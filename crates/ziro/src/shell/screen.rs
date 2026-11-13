use super::popup::{Popup, Rect, restore};
use ziro_proto::{Message, ProtocolError};

#[derive(Default, Debug)]
pub struct Boundary {
    phase: u8,
    escape: bool,
    utf8: u8,
    csi: Vec<u8>,
}
impl Boundary {
    pub fn ready(&self) -> bool {
        self.phase == 0 && self.utf8 == 0
    }
    fn byte(&mut self, b: u8) -> Option<Vec<u8>> {
        if self.phase == 0 {
            if self.utf8 > 0 {
                if b & 0xc0 == 0x80 {
                    self.utf8 -= 1;
                    return None;
                }
                self.utf8 = 0;
            }
            if b == 27 {
                self.phase = 1;
            } else if (0xc2..=0xdf).contains(&b) {
                self.utf8 = 1;
            } else if (0xe0..=0xef).contains(&b) {
                self.utf8 = 2;
            } else if (0xf0..=0xf4).contains(&b) {
                self.utf8 = 3;
            }
        } else if self.phase == 1 {
            self.phase = match b {
                b'[' => {
                    self.csi.clear();
                    2
                }
                b']' => 3,
                b'P' | b'_' | b'^' => 4,
                0x20..=0x2f => 5,
                _ => 0,
            };
            self.escape = false;
        } else if self.phase == 2 {
            if b == 24 || b == 26 {
                self.phase = 0;
            } else if (0x40..=0x7e).contains(&b) {
                self.phase = 0;
                self.csi.push(b);
                return Some(std::mem::take(&mut self.csi));
            } else if self.csi.len() < 128 {
                self.csi.push(b);
            } else {
                self.phase = 0;
            }
        } else if self.phase == 3 || self.phase == 4 {
            if (self.phase == 3 && b == 7) || (self.escape && b == b'\\') || b == 24 || b == 26 {
                self.phase = 0;
                self.escape = false;
            } else {
                self.escape = b == 27;
            }
        } else if !(0x20..=0x2f).contains(&b) {
            self.phase = 0;
        }
        None
    }
}
#[derive(Default)]
pub struct Ui {
    pub popup: Option<Popup>,
    pub generation: u64,
    pub editing: bool,
}
impl Ui {
    pub fn apply(&mut self, m: &Message) -> Result<(), ProtocolError> {
        match m.kind.as_str() {
            "CTX" => {
                let generation = m.number::<u64>(0)?;
                if generation >= self.generation {
                    self.generation = generation;
                    self.editing = !m.field(7).is_empty()
                        && m.field(4) != "vicmd"
                        && !m.field(6).contains('h');
                    if !self.editing {
                        self.popup = None;
                    }
                }
            }
            "FRAME" if m.number::<u64>(0)? == self.generation && self.editing => {
                self.popup = Popup::from_message(m)?
            }
            "UI_CLEAR" if m.number::<u64>(0)? >= self.generation => self.popup = None,
            "UI_STATE" => {
                self.editing = m.field(0) == "1";
                if !self.editing {
                    self.popup = None;
                }
            }
            "UI_SELECT" if m.number::<u64>(0)? == self.generation => {
                if let Some(p) = &mut self.popup
                    && p.generation == self.generation
                {
                    p.selected = m
                        .number::<usize>(1)?
                        .min(p.desired_height().saturating_sub(3));
                }
            }
            _ => {}
        }
        Ok(())
    }
}
struct Paint {
    rect: Rect,
    draw: String,
    restore: String,
}
pub struct Compositor {
    pub parser: vt100::Parser,
    boundary: Boundary,
    pending: Vec<u8>,
    paint: Option<Paint>,
    pub ui: Ui,
    known_from: u16,
    sync_supported: bool,
    child_sync: bool,
    partial_forwarded: bool,
}
impl Compositor {
    pub fn new(rows: u16, cols: u16, origin: (u16, u16), sync_supported: bool) -> Self {
        let mut parser = vt100::Parser::new(
            rows.max(1).try_into().unwrap(),
            cols.max(1).try_into().unwrap(),
            256,
        );
        parser.process(format!("\x1b[{};{}H", origin.0 + 1, origin.1 + 1).as_bytes());
        Self {
            parser,
            boundary: Boundary::default(),
            pending: vec![],
            paint: None,
            ui: Ui::default(),
            known_from: origin.0,
            sync_supported,
            child_sync: false,
            partial_forwarded: false,
        }
    }
    fn feed(&mut self, bytes: &[u8]) {
        let mut start = 0;
        let before = self.parser.screen().scrollback_len();
        for (i, &b) in bytes.iter().enumerate() {
            if let Some(csi) = self.boundary.byte(b) {
                self.parser.process(&bytes[start..=i]);
                start = i + 1;
                if csi == b"?2026h" {
                    self.child_sync = true;
                } else if csi == b"?2026l" {
                    self.child_sync = false;
                }
                if !self.parser.screen().alternate_screen() {
                    if csi == b"2J" {
                        self.known_from = 0;
                    } else if (csi == b"J" || csi == b"0J")
                        && self.parser.screen().cursor_position().1 == 0
                    {
                        self.known_from = self
                            .known_from
                            .min(self.parser.screen().cursor_position().0);
                    }
                }
            }
        }
        self.parser.process(&bytes[start..]);
        let scrolled = self.parser.screen().scrollback_len().saturating_sub(before);
        self.known_from = self
            .known_from
            .saturating_sub(scrolled.min(u16::MAX as usize) as u16);
    }
    pub fn push_child(&mut self, bytes: &[u8]) {
        self.feed(bytes);
        self.pending.extend_from_slice(bytes);
    }
    pub fn apply(&mut self, m: &Message) {
        if self.ui.apply(m).is_err() {
            self.ui.popup = None;
        }
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Vec<u8> {
        // Restore only the part still on screen; never wrap old-width rows.
        let mut output = String::new();
        if let Some(old) = self.paint.take()
            && self.boundary.ready()
        {
            let mut rect = old.rect;
            rect.height = rect.height.min(rows.saturating_sub(rect.row));
            rect.width = rect.width.min(cols.saturating_sub(rect.col + 1));
            if rect.height > 0 && rect.width > 0 {
                output = restore(rect, self.parser.screen());
            }
        }
        let old_rows = self.parser.screen().size().0.get();
        self.parser.set_size(
            rows.max(1).try_into().unwrap(),
            cols.max(1).try_into().unwrap(),
        );
        self.known_from =
            (self.known_from + rows.saturating_sub(old_rows)).min(rows.saturating_sub(1));
        self.ui.popup = None;
        output.into_bytes()
    }
    pub fn flush(&mut self) -> Vec<u8> {
        if !self.boundary.ready() {
            if self.pending.len() <= 1024 * 1024 {
                return vec![];
            }
            let mut out = self
                .paint
                .take()
                .map(|p| p.restore.into_bytes())
                .unwrap_or_default();
            out.append(&mut self.pending);
            self.partial_forwarded = true;
            return out;
        }
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let (rows, cols) = (rows.get(), cols.get());
        let active = self.ui.editing && !screen.alternate_screen() && !self.child_sync;
        if active
            && let Some(popup) = &self.ui.popup
            && popup
                .place(rows, cols, screen.cursor_position(), self.known_from)
                .is_none()
        {
            // Do not erase terminal history that predates the wrapper. Make
            // room by scrolling it into scrollback, rather than painting over
            // unknown cells and later restoring blanks.
            let (row, col) = screen.cursor_position();
            let count = (popup.desired_height() as u16)
                .min(rows.saturating_sub(1))
                .min(row);
            if count > 0 {
                let sequence = format!(
                    "\x1b[{};1H{}\x1b[{};{}H",
                    rows,
                    "\r\n".repeat(count as usize),
                    row - count + 1,
                    col + 1
                );
                self.feed(sequence.as_bytes());
                self.pending.extend_from_slice(sequence.as_bytes());
            }
        }
        let screen = self.parser.screen();
        let new = if active {
            self.ui.popup.as_ref().and_then(|p| {
                p.place(rows, cols, screen.cursor_position(), self.known_from)
                    .map(|rect| Paint {
                        rect,
                        draw: p.draw(rect, screen),
                        restore: restore(rect, screen),
                    })
            })
        } else {
            None
        };
        if self.pending.is_empty()
            && match (&self.paint, &new) {
                (Some(a), Some(b)) => a.rect == b.rect && a.draw == b.draw,
                (None, None) => true,
                _ => false,
            }
        {
            return vec![];
        }
        let had_overlay = self.paint.is_some() || new.is_some();
        let mut output = vec![];
        let synchronized =
            self.sync_supported && had_overlay && !self.child_sync && !self.partial_forwarded;
        if synchronized {
            output.extend_from_slice(b"\x1b[?2026h");
        }
        if let Some(old) = self.paint.take() {
            output.extend_from_slice(old.restore.as_bytes());
        }
        output.append(&mut self.pending);
        if let Some(new) = &new {
            output.extend_from_slice(new.draw.as_bytes());
        }
        if synchronized {
            output.extend_from_slice(b"\x1b[?2026l");
        }
        self.paint = new;
        self.partial_forwarded = false;
        output
    }
    pub fn close(&mut self) -> Vec<u8> {
        self.ui.editing = false;
        self.ui.popup = None;
        self.flush()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ziro_core::{
        candidate::{Candidate, Context, Kind},
        config::Config,
        layout,
        rank::Query,
    };
    fn show(c: &mut Compositor) {
        c.apply(&Message::new(
            "CTX",
            ["1", "4", "80", "20", "main", "/tmp", "", "cd D"],
        ));
        let mut row = Candidate::history("Documents/".into(), 0.);
        row.kind = Kind::Directory;
        let frame = layout::layout(
            1,
            "cd D",
            &Query::parse("cd D", 4),
            &Context::default(),
            &[row],
            0,
            "",
            80,
            &Config::default(),
        );
        c.apply(&frame.message());
    }
    #[test]
    fn cannot_interleave_controls_or_utf8() {
        let mut c = Compositor::new(20, 80, (0, 0), true);
        show(&mut c);
        c.push_child(b"\x1b[");
        assert!(c.flush().is_empty());
        c.push_child(b"31m");
        assert!(!c.flush().is_empty());
        c.push_child(&[0xe7, 0x9b]);
        assert!(c.flush().is_empty());
        c.push_child(&[0xae]);
        assert!(!c.flush().is_empty());
    }
    #[test]
    fn restores_overlay_and_never_takes_over_alternate_screen() {
        let mut c = Compositor::new(20, 80, (0, 0), true);
        let mut actual = vt100::Parser::new(20.try_into().unwrap(), 80.try_into().unwrap(), 0);
        c.push_child(b"\x1b[2J\x1b[7;1HBASE\x1b[4;8H");
        actual.process(&c.flush());
        show(&mut c);
        actual.process(&c.flush());
        assert!(actual.screen().contents().contains("Documents"));
        assert!(c.flush().is_empty());
        actual.process(&c.close());
        assert_eq!(actual.screen().contents(), c.parser.screen().contents());
        c.push_child(b"\x1b[?1049h\x1b[2Jvim");
        show(&mut c);
        let raw = c.flush();
        actual.process(&raw);
        assert!(!actual.screen().contents().contains("Documents"));
        assert!(actual.screen().alternate_screen());
    }
    #[test]
    fn unknown_history_is_not_overwritten() {
        let mut c = Compositor::new(10, 80, (9, 0), true);
        show(&mut c);
        let output = c.flush();
        assert!(output.windows(2).any(|s| s == b"\r\n"));
        assert!(c.paint.is_some());
        assert_eq!(c.parser.screen().cursor_position().0, 6);
    }
    #[test]
    fn generation_and_clear() {
        let mut c = Compositor::new(20, 80, (0, 0), false);
        show(&mut c);
        c.flush();
        c.apply(&Message::new(
            "CTX",
            ["2", "5", "80", "20", "main", "/tmp", "", "cd Do"],
        ));
        c.apply(&Message::new("UI_CLEAR", ["1"]));
        assert!(c.ui.popup.is_some());
        c.apply(&Message::new("UI_STATE", ["0"]));
        assert!(c.ui.popup.is_none());
    }
}

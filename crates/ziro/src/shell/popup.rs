use std::fmt::Write as _;
use unicode_width::UnicodeWidthStr;
use vt100::Color;
use ziro_proto::{Message, ProtocolError};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}
impl Style {
    pub fn from_spec(spec: &str) -> Self {
        let mut style = Self::default();
        for part in spec.split(',') {
            if let Some(value) = part.strip_prefix("fg=") {
                style.fg = color(value);
            } else if let Some(value) = part.strip_prefix("bg=") {
                style.bg = color(value);
            } else {
                match part {
                    "bold" => style.bold = true,
                    "dim" => style.dim = true,
                    "italic" => style.italic = true,
                    "underline" => style.underline = true,
                    "standout" | "reverse" => style.inverse = true,
                    _ => {}
                }
            }
        }
        style
    }
    pub fn merge(self, other: Self) -> Self {
        Self {
            fg: other.fg.or(self.fg),
            bg: other.bg.or(self.bg),
            bold: self.bold || other.bold,
            dim: self.dim || other.dim,
            italic: self.italic || other.italic,
            underline: self.underline || other.underline,
            inverse: self.inverse || other.inverse,
        }
    }
    pub fn ansi(self) -> String {
        let mut codes = vec!["0".to_owned()];
        for (on, code) in [
            (self.bold, 1),
            (self.dim, 2),
            (self.italic, 3),
            (self.underline, 4),
            (self.inverse, 7),
        ] {
            if on {
                codes.push(code.to_string());
            }
        }
        for (value, fg) in [(self.fg, true), (self.bg, false)] {
            if let Some(value) = value {
                match value {
                    Color::Default => codes.push(if fg { "39" } else { "49" }.into()),
                    Color::Idx(n) => codes.push(format!("{};5;{n}", if fg { 38 } else { 48 })),
                    Color::Rgb(r, g, b) => {
                        codes.push(format!("{};2;{r};{g};{b}", if fg { 38 } else { 48 }))
                    }
                }
            }
        }
        format!("\x1b[{}m", codes.join(";"))
    }
    pub fn from_cell(cell: &vt100::Cell) -> Self {
        Self {
            fg: Some(cell.fgcolor()),
            bg: Some(cell.bgcolor()),
            bold: cell.bold(),
            dim: cell.dim(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
        }
    }
}
fn color(s: &str) -> Option<Color> {
    if s == "default" {
        return Some(Color::Default);
    }
    if let Some(hex) = s.strip_prefix('#')
        && hex.len() == 6
        && let Ok(value) = u32::from_str_radix(hex, 16)
    {
        return Some(Color::Rgb(
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        ));
    }
    s.parse::<u8>().ok().map(Color::Idx)
}
#[derive(Clone, Debug)]
pub struct Line {
    pub text: String,
    styles: Vec<Style>,
}
impl Line {
    fn render(&self, width: usize, selected: Option<Style>) -> String {
        let mut output = String::new();
        let mut visible = String::new();
        let mut previous = None;
        for (i, c) in self.text.chars().enumerate() {
            visible.push(c);
            if visible.width() > width {
                visible.pop();
                break;
            }
            let style = self
                .styles
                .get(i)
                .copied()
                .unwrap_or_default()
                .merge(selected.unwrap_or_default());
            if previous != Some(style) {
                output.push_str(&style.ansi());
                previous = Some(style);
            }
            output.push(c);
        }
        let style = selected.unwrap_or_default();
        if previous != Some(style) {
            output.push_str(&style.ansi());
        }
        output.push_str(&" ".repeat(width.saturating_sub(visible.width())));
        output
    }
}
#[derive(Debug, Clone)]
pub struct Popup {
    pub generation: u64,
    pub selected: usize,
    top: Option<Line>,
    items: Vec<Line>,
    bottom: Option<Line>,
    selected_style: Style,
    pub width: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub row: u16,
    pub col: u16,
    pub width: u16,
    pub height: u16,
}
impl Popup {
    pub fn from_message(m: &Message) -> Result<Option<Self>, ProtocolError> {
        m.validate()?;
        let n = m.number::<usize>(3)?;
        if n == 0 {
            return Ok(None);
        }
        let generation = m.number(0)?;
        let ghost = m.number::<usize>(1)?;
        let selected = m.number::<usize>(2)?;
        let highlights = m.number::<usize>(5)?;
        let text: Vec<char> = m.field(4).chars().collect();
        let mut rows = vec![];
        for i in 0..n {
            let base = 6 + highlights + i * 4;
            let a = m.number::<usize>(base)?;
            let b = m.number::<usize>(base + 1)?;
            if a > b || b > text.len() {
                return Err(ProtocolError("invalid popup row".into()));
            }
            rows.push((a, b));
        }
        let indent = text[rows[0].0..rows[0].1]
            .iter()
            .take_while(|&&c| c == ' ')
            .count();
        let mut spans = vec![];
        let mut selected_style = Style::from_spec("fg=15,bg=4");
        for spec in m.fields.iter().skip(6).take(highlights) {
            let mut fields = spec.splitn(3, ' ');
            let a = fields.next().and_then(|v| v.parse::<usize>().ok());
            let b = fields.next().and_then(|v| v.parse::<usize>().ok());
            let style = Style::from_spec(fields.next().unwrap_or(""));
            if let (Some(a), Some(b)) = (a, b) {
                if (a, b) == rows[selected] {
                    selected_style = style;
                } else {
                    spans.push((a, b, style));
                }
            }
        }
        let line = |a: usize, b: usize| {
            let a = (a + indent).min(b);
            let content: String = text[a..b]
                .iter()
                .map(|&c| if c.is_control() { ' ' } else { c })
                .collect();
            let styles = (a..b)
                .map(|i| {
                    spans
                        .iter()
                        .filter(|(a, b, _)| i >= *a && i < *b)
                        .fold(Style::default(), |s, (_, _, v)| s.merge(*v))
                })
                .collect();
            Line {
                text: content,
                styles,
            }
        };
        let items = rows.iter().map(|&(a, b)| line(a, b)).collect::<Vec<_>>();
        let menu_start = (ghost + 1).min(text.len());
        let top_end = rows[0].0.saturating_sub(1);
        let bottom_start = (rows[n - 1].1 + 1).min(text.len());
        let top = if menu_start < top_end {
            Some(line(menu_start, top_end))
        } else {
            None
        };
        let bottom = if bottom_start < text.len() {
            Some(line(bottom_start, text.len()))
        } else {
            None
        };
        let width = items
            .iter()
            .map(|s| s.text.width())
            .chain(top.iter().map(|s| s.text.width()))
            .chain(bottom.iter().map(|s| s.text.width()))
            .max()
            .unwrap_or(0);
        Ok(Some(Self {
            generation,
            selected,
            top,
            items,
            bottom,
            selected_style,
            width,
        }))
    }
    pub fn desired_height(&self) -> usize {
        self.items.len() + usize::from(self.top.is_some()) + usize::from(self.bottom.is_some())
    }
    pub fn minimum_height(&self) -> usize {
        1 + usize::from(self.top.is_some()) + usize::from(self.bottom.is_some())
    }
    pub fn place(&self, rows: u16, cols: u16, cursor: (u16, u16), known_from: u16) -> Option<Rect> {
        let below = rows.saturating_sub(cursor.0 + 1) as usize;
        let above = cursor.0.saturating_sub(known_from) as usize;
        let height = self.desired_height().min((rows as usize).saturating_sub(1));
        let (row, height) = if below >= height {
            (cursor.0 + 1, height)
        } else if above >= height {
            (cursor.0 - height as u16, height)
        } else if below >= above {
            (cursor.0 + 1, below)
        } else {
            (cursor.0 - above as u16, above)
        };
        if height < self.minimum_height() || cols < 5 {
            return None;
        }
        let width = self.width.min(cols.saturating_sub(1) as usize) as u16;
        Some(Rect {
            row,
            col: cursor.1.min(cols.saturating_sub(width + 1)),
            width,
            height: height as u16,
        })
    }
    pub fn draw(&self, rect: Rect, screen: &vt100::Screen) -> String {
        let mut output = "\x1b[?25l".to_owned();
        let border = usize::from(self.top.is_some()) + usize::from(self.bottom.is_some());
        let count = (rect.height as usize)
            .saturating_sub(border)
            .min(self.items.len());
        let first = self
            .selected
            .saturating_sub(count / 2)
            .min(self.items.len() - count);
        let mut lines: Vec<(&Line, Option<Style>)> = vec![];
        if let Some(top) = &self.top {
            lines.push((top, None));
        }
        lines.extend(
            self.items
                .iter()
                .enumerate()
                .skip(first)
                .take(count)
                .map(|(i, line)| {
                    (
                        line,
                        if i == self.selected {
                            Some(self.selected_style)
                        } else {
                            None
                        },
                    )
                }),
        );
        if let Some(bottom) = &self.bottom {
            lines.push((bottom, None));
        }
        for (i, (line, style)) in lines.iter().enumerate() {
            let _ = write!(output, "\x1b[{};{}H", rect.row + 1 + i as u16, rect.col + 1);
            output.push_str(&line.render(rect.width as usize, *style));
        }
        output.push_str(&screen.cursor_state_formatted());
        output.push_str(&screen.attributes_formatted());
        output
    }
}
pub fn restore(rect: Rect, screen: &vt100::Screen) -> String {
    let mut output = "\x1b[?25l".to_owned();
    for row in rect.row..rect.row + rect.height {
        let _ = write!(output, "\x1b[{};{}H", row + 1, rect.col + 1);
        let mut previous = None;
        for col in rect.col..rect.col + rect.width {
            if let Some(cell) = screen.cell(row, col) {
                if cell.is_wide_continuation() {
                    continue;
                }
                let style = Style::from_cell(cell);
                if previous != Some(style) {
                    output.push_str(&style.ansi());
                    previous = Some(style);
                }
                if cell.has_contents() {
                    output.push_str(cell.contents());
                } else {
                    output.push(' ');
                }
            }
        }
    }
    output.push_str(&screen.cursor_state_formatted());
    output.push_str(&screen.attributes_formatted());
    output
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ziro_core::{
        candidate::{Candidate, Context, Kind},
        config::Config,
        layout,
        rank::Query,
    };
    fn popup() -> Popup {
        let mut c = Candidate::history("Documents/".into(), 0.);
        c.kind = Kind::Directory;
        let frame = layout::layout(
            1,
            "cd D",
            &Query::parse("cd D", 4),
            &Context::default(),
            &[c],
            0,
            "",
            100,
            &Config::default(),
        );
        Popup::from_message(&frame.message()).unwrap().unwrap()
    }
    /// Visual comparison: an explicit blank with default style is
    /// indistinguishable on a terminal from a never-written cell.
    pub fn visual(screen: &vt100::Screen) -> Vec<String> {
        let (rows, cols) = screen.size();
        (0..rows.get())
            .map(|r| {
                let mut line = String::new();
                for c in 0..cols.get() {
                    let cell = screen.cell(r, c).unwrap();
                    let text = if cell.has_contents() {
                        cell.contents()
                    } else {
                        " "
                    };
                    let style = Style::from_cell(cell);
                    if style == Style::from_spec("fg=default,bg=default") && text == " " {
                        line.push(' ');
                    } else {
                        line.push_str(&format!("{text}{:?}", style));
                    }
                }
                line.trim_end().to_owned()
            })
            .collect()
    }
    #[test]
    fn moves_with_cursor_and_flips_above() {
        let p = popup();
        assert_eq!(p.place(30, 100, (2, 14), 0).unwrap().row, 3);
        assert_eq!(p.place(30, 100, (29, 20), 0).unwrap().row, 26);
        assert_eq!(p.place(30, 100, (2, 14), 0).unwrap().col, 14);
        assert!(p.place(30, 100, (29, 20), 29).is_none());
        let r = p.place(30, 30, (2, 28), 0).unwrap();
        assert!(r.col + r.width < 30);
    }
    #[test]
    fn restore_preserves_text_style_cursor_and_application_save_slot() {
        let mut base = vt100::Parser::new(20.try_into().unwrap(), 80.try_into().unwrap(), 0);
        base.process(b"\x1b[2J\x1b[3;1H\x1b[31mBACKGROUND\x1b7\x1b[6;10H");
        let p = popup();
        let rect = p.place(20, 80, (6, 10), 0).unwrap();
        let original = base.screen().clone();
        let restore_bytes = restore(rect, &original);
        let draw = p.draw(rect, &original);
        base.process(draw.as_bytes());
        assert!(base.screen().contents().contains("Documents"));
        base.process(restore_bytes.as_bytes());
        assert_eq!(visual(base.screen()), visual(&original));
        assert_eq!(base.screen().cursor_position(), original.cursor_position());
        base.process(b"\x1b8");
        assert_eq!(base.screen().cursor_position(), (2, 10));
    }
}

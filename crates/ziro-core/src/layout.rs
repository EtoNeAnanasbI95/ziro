use crate::{
    apply::{Applied, apply_candidate},
    candidate::{Candidate, Context, Kind},
    config::Config,
    rank::Query,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use ziro_proto::Message;
#[derive(Debug, Clone)]
pub struct Row {
    pub start: usize,
    pub end: usize,
    pub applied: Applied,
}
#[derive(Debug, Clone)]
pub struct Frame {
    pub generation: u64,
    pub ghost_len: usize,
    pub selected: usize,
    pub postdisplay: String,
    pub highlights: Vec<String>,
    pub rows: Vec<Row>,
}
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() || c as u32 >= 0x1f000 {
                ' '
            } else {
                c
            }
        })
        .collect()
}
fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w >= width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}
#[allow(clippy::too_many_arguments)]
pub fn layout(
    generation: u64,
    buffer: &str,
    query: &Query,
    context: &Context,
    candidates: &[Candidate],
    selected: usize,
    ghost: &str,
    cols: usize,
    config: &Config,
) -> Frame {
    let ghost = ghost.to_owned();
    let mut f = Frame {
        generation,
        ghost_len: ghost.chars().count(),
        selected: 0,
        postdisplay: ghost,
        highlights: vec![],
        rows: vec![],
    };
    if f.ghost_len > 0 {
        f.highlights
            .push(format!("0 {} {}", f.ghost_len, config.theme.ghost));
    }
    if candidates.is_empty() || cols < 5 {
        return f;
    }
    let selected = selected.min(candidates.len() - 1);
    let count = config.menu.max_items.min(candidates.len());
    let first = selected
        .saturating_sub(count / 2)
        .min(candidates.len() - count);
    f.selected = selected - first;
    let width = cols.saturating_sub(1).min(78);
    let inner = width.saturating_sub(4);
    let before: String = buffer.chars().take(query.start).collect();
    let indent = before.width().min(cols.saturating_sub(width + 1));
    let pad = " ".repeat(indent);

    let border = config.menu.border != "none";
    let (tl, tr, bl, br, h, v) = if config.menu.border == "sharp" {
        ("┌", "┐", "└", "┘", "─", "│")
    } else {
        ("╭", "╮", "╰", "╯", "─", "│")
    };
    if border {
        let start = f.postdisplay.chars().count() + 1;
        f.postdisplay.push_str(&format!(
            "\n{pad}{tl}{}{tr}",
            h.repeat(width.saturating_sub(2))
        ));
        f.highlights.push(format!(
            "{start} {} {}",
            f.postdisplay.chars().count(),
            config.theme.border
        ));
    }
    for (i, c) in candidates.iter().enumerate().skip(first).take(count) {
        let icon = if config.menu.icons == "none" {
            ""
        } else {
            match c.kind {
                Kind::Directory => "/ ",
                Kind::Branch => "⎇ ",
                Kind::History => "↺ ",
                Kind::Command => "› ",
                _ => "  ",
            }
        };
        let display = clean(&c.display);
        let desc = clean(&c.description);
        let label = if config.menu.show_descriptions && !desc.is_empty() {
            format!("{icon}{display}  {desc}")
        } else {
            format!("{icon}{display}")
        };
        let label = truncate(&label, inner);
        let padding = " ".repeat(inner.saturating_sub(label.width()));
        f.postdisplay.push('\n');
        let start = f.postdisplay.chars().count();
        f.postdisplay.push_str(&if border {
            format!("{pad}{v} {label}{padding} {v}")
        } else {
            format!("{pad}{label}")
        });
        let end = f.postdisplay.chars().count();
        let text_start = start + indent + if border { 2 } else { 0 };
        let label_len = label.chars().count();
        let desc_offset = icon.chars().count() + display.chars().count() + 2;
        if config.menu.show_descriptions && !desc.is_empty() && desc_offset < label_len {
            f.highlights.push(format!(
                "{} {} {}",
                text_start + desc_offset,
                text_start + label_len,
                config.theme.description
            ));
        }
        let needle = query
            .word
            .strip_prefix(&context.iprefix)
            .unwrap_or(&query.word);
        let positions = crate::rank::match_positions(&c.word, &c.prefix, needle);
        for &index in &positions {
            let offset = icon.chars().count() + index as usize;
            if offset < label_len {
                f.highlights.push(format!(
                    "{} {} {}",
                    text_start + offset,
                    text_start + offset + 1,
                    config.theme.r#match
                ));
            }
        }
        if border {
            f.highlights.push(format!(
                "{} {} {}",
                start + indent,
                start + indent + 1,
                config.theme.border
            ));
            f.highlights
                .push(format!("{} {} {}", end - 1, end, config.theme.border));
        }
        if i == selected {
            f.highlights
                .push(format!("{start} {end} {}", config.theme.selected));
        }
        f.rows.push(Row {
            start,
            end,
            applied: apply_candidate(buffer, query, context, c),
        });
    }
    if border {
        let start = f.postdisplay.chars().count() + 1;
        let counter = format!(" {}/{} ", selected + 1, candidates.len());
        let counter = truncate(&counter, width.saturating_sub(2));
        f.postdisplay.push_str(&format!(
            "\n{pad}{bl}{}{counter}{br}",
            h.repeat(width.saturating_sub(2 + counter.width()))
        ));
        f.highlights.push(format!(
            "{start} {} {}",
            f.postdisplay.chars().count(),
            config.theme.border
        ));
    }
    f
}
impl Frame {
    pub fn message(&self) -> Message {
        let mut fields = vec![
            self.generation.to_string(),
            self.ghost_len.to_string(),
            self.selected.to_string(),
            self.rows.len().to_string(),
            self.postdisplay.clone(),
            self.highlights.len().to_string(),
        ];
        fields.extend(self.highlights.clone());
        for r in &self.rows {
            fields.extend([
                r.start.to_string(),
                r.end.to_string(),
                r.applied.buffer.clone(),
                r.applied.cursor.to_string(),
            ]);
        }
        Message::new("FRAME", fields)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn widths_and_character_offsets() {
        for cols in [5, 20, 40, 200] {
            let mut c = Candidate::history("目录/файл🦀".repeat(20), 0.);
            c.kind = Kind::File;
            let q = Query::parse("cat ", 4);
            let f = layout(
                1,
                "cat ",
                &q,
                &Context::default(),
                &[c],
                0,
                "",
                cols,
                &Config::default(),
            );
            for line in f.postdisplay.lines() {
                assert!(line.width() < cols, "{cols}: {line}");
            }
            for r in f.rows {
                assert!(r.end <= f.postdisplay.chars().count());
            }
        }
    }
}

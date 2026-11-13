use crate::{
    candidate::{Candidate, Context, Kind},
    rank::Query,
};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub buffer: String,
    pub cursor: usize,
}
fn escape(word: &str, quote: Option<char>) -> String {
    let mut out = String::new();
    for c in word.chars() {
        match quote {
            None if c == '\n' => out.push_str("$'\\n'"),
            Some('\'') if c == '\'' => out.push_str("'\\''"),
            Some('"') if matches!(c, '"' | '\\' | '$' | '`') => {
                out.push('\\');
                out.push(c);
            }
            None if !(c.is_alphanumeric() || "/_-.=,:+@%".contains(c)) => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}
pub fn apply_candidate(
    buffer: &str,
    query: &Query,
    context: &Context,
    candidate: &Candidate,
) -> Applied {
    if candidate.kind == Kind::History {
        return Applied {
            buffer: candidate.word.clone(),
            cursor: candidate.word.chars().count(),
        };
    }
    let original: Vec<char> = buffer.chars().collect();
    let mut inserted = String::new();
    let raw: String = original[query.start..query.cursor].iter().collect();
    let quote = raw.chars().next().filter(|c| matches!(c, '\'' | '"'));
    if let Some(q) = quote {
        inserted.push(q);
    }
    inserted.push_str(&context.iprefix);
    inserted.push_str(&candidate.prefix);
    inserted.push_str(&escape(&candidate.word, quote));
    inserted.push_str(&candidate.suffix);
    inserted.push_str(&context.isuffix);
    if let Some(q) = quote {
        inserted.push(q);
    }
    let add_space = candidate.suffix.is_empty()
        && context.isuffix.is_empty()
        && candidate.kind != Kind::Directory
        && !candidate.flags.contains('q');
    if add_space {
        inserted.push(' ');
    }
    let cursor = query.start + inserted.chars().count();
    let before: String = original[..query.start].iter().collect();
    let after: String = original[query.end..].iter().collect();
    Applied {
        buffer: format!("{before}{inserted}{after}"),
        cursor,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn insert(input: &str, cursor: usize, word: &str, kind: Kind) -> Applied {
        let mut c = Candidate::history(word.into(), 0.);
        c.kind = kind;
        apply_candidate(input, &Query::parse(input, cursor), &Context::default(), &c)
    }
    #[test]
    fn escaping_and_middle() {
        assert_eq!(
            insert("cat M", 5, "My Docs/", Kind::Directory).buffer,
            "cat My\\ Docs/"
        );
        assert_eq!(
            insert("cat 'My", 7, "My Docs/", Kind::Directory).buffer,
            "cat 'My Docs/'"
        );
        assert_eq!(
            insert("cat ф", 5, "файл'1.txt", Kind::File).buffer,
            "cat файл\\'1.txt "
        );
        assert_eq!(
            insert("ls --c tail", 6, "--color=auto", Kind::Option).buffer,
            "ls --color=auto  tail"
        );
        assert_eq!(
            insert("cat old.txt suffix", 6, "new", Kind::File).buffer,
            "cat new  suffix"
        );
    }
}

#[cfg(test)]
mod literal_tests {
    #[test]
    fn newlines_are_not_shell_continuations() {
        for word in [
            "a\nb",
            "файл с пробелом'1",
            "a\tb",
            "$(echo unsafe)",
            "a\\b",
        ] {
            let quoted = super::escape(word, None);
            let output = std::process::Command::new("zsh")
                .arg("-fc")
                .arg(format!("print -rn -- {quoted}"))
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, word.as_bytes());
        }
    }
}

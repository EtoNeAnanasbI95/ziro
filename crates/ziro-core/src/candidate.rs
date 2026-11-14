use ziro_proto::Message;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Command,
    Subcommand,
    Alias,
    Function,
    Builtin,
    Keyword,
    Option,
    Directory,
    File,
    Branch,
    History,
    Argument,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Command => "binary",
            Self::Subcommand => "subcommand",
            Self::Alias => "alias",
            Self::Function => "function",
            Self::Builtin => "builtin",
            Self::Keyword => "keyword",
            Self::Directory => "directory",
            Self::File => "file",
            Self::Branch => "branch",
            Self::History => "history",
            Self::Option => "option",
            Self::Argument => "argument",
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub prefix: String,
    pub suffix: String,
    pub iprefix: String,
    pub isuffix: String,
    pub quote: String,
}
/// Shared immutable text: candidates are re-ranked on every keystroke, and
/// cloning eight owned strings per candidate dominated ranking time.
pub type Text = std::sync::Arc<str>;
#[derive(Debug, Clone)]
pub struct Candidate {
    pub group: Text,
    pub tag: Text,
    pub word: Text,
    pub display: Text,
    pub description: Text,
    pub flags: Text,
    pub prefix: Text,
    pub suffix: Text,
    pub kind: Kind,
    pub score: f64,
    pub matches: Vec<u32>,
}
pub fn split_description(text: &str) -> (&str, &str) {
    text.split_once("--")
        .map(|(a, b)| (a.trim(), b.trim()))
        .unwrap_or((text.trim(), ""))
}
pub fn kind_of(tag: &str, flags: &str, word: &str) -> Kind {
    if tag.contains("director") || word.ends_with('/') {
        Kind::Directory
    } else if tag.contains("branch") || tag.contains("heads-") {
        Kind::Branch
    } else if word.starts_with('-') || tag.contains("option") {
        Kind::Option
    } else if tag.contains("alias") {
        Kind::Alias
    } else if tag.contains("function") {
        Kind::Function
    } else if tag.contains("builtin") {
        Kind::Builtin
    } else if tag.contains("reserved-word") {
        Kind::Keyword
    } else if tag.contains("subcommand") {
        Kind::Subcommand
    } else if tag.contains("command") || tag.contains("executable") {
        Kind::Command
    } else if flags.contains('f') || tag.contains("file") {
        Kind::File
    } else {
        Kind::Argument
    }
}
impl Candidate {
    pub fn from_record(m: &Message) -> Self {
        let (display, desc) = split_description(m.field(3));
        Self {
            group: m.field(0).into(),
            tag: m.field(1).into(),
            word: m.field(2).into(),
            display: if display.is_empty() {
                m.field(2).into()
            } else {
                display.into()
            },
            description: if m.field(4).is_empty() {
                desc.into()
            } else {
                m.field(4).into()
            },
            flags: m.field(5).into(),
            prefix: m.field(6).into(),
            suffix: m.field(7).into(),
            kind: kind_of(m.field(1), m.field(5), m.field(2)),
            score: 0.,
            matches: vec![],
        }
    }
    pub fn history(word: String, score: f64) -> Self {
        Self {
            group: "history".into(),
            tag: "history".into(),
            display: word.as_str().into(),
            word: word.into(),
            description: "history".into(),
            flags: "".into(),
            prefix: "".into(),
            suffix: "".into(),
            kind: Kind::History,
            score,
            matches: vec![],
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptions_and_kinds() {
        assert_eq!(
            split_description("checkout -- Switch branches -- safely"),
            ("checkout", "Switch branches -- safely")
        );
        for (tag, flags, word, want) in [
            ("directories", "", "foo", Kind::Directory),
            ("", "", "a/", Kind::Directory),
            ("heads", "", "main", Kind::Argument),
            ("branches", "", "main", Kind::Branch),
            ("", "", "--color=auto", Kind::Option),
            ("files", "f", "x", Kind::File),
            ("commands", "", "git", Kind::Command),
            ("aliases", "", "g", Kind::Alias),
            ("functions", "", "deploy", Kind::Function),
            ("builtins", "", "cd", Kind::Builtin),
            ("reserved-words", "", "if", Kind::Keyword),
        ] {
            assert_eq!(kind_of(tag, flags, word), want);
        }
    }
}
#[cfg(test)]
mod fixture_tests {
    #[test]
    fn real_compsys_descriptions() {
        let fixtures: std::collections::BTreeMap<String, Vec<String>> =
            serde_json::from_str(include_str!("../../../spikes/fixtures/descriptions.json"))
                .unwrap();
        for descriptions in fixtures.values() {
            for display in descriptions {
                let (word, description) = super::split_description(display);
                assert!(!word.is_empty());
                if display.contains("--") {
                    assert!(!description.is_empty());
                }
            }
        }
        let checkout = &fixtures["git ch"][0];
        assert!(super::split_description(checkout).1.contains("branch"));
    }
}

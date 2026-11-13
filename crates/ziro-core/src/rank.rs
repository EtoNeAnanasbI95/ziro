use crate::{
    candidate::{Candidate, Kind},
    config::Config,
    history::Entry,
};
use std::collections::HashMap;
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub words: Vec<String>,
    pub word: String,
    pub start: usize,
    pub end: usize,
    pub cursor: usize,
    pub quote: Option<char>,
}
impl Query {
    pub fn parse(buffer: &str, cursor: usize) -> Self {
        let chars: Vec<char> = buffer.chars().collect();
        let cursor = cursor.min(chars.len());
        let mut words = vec![];
        let mut word = String::new();
        let mut quote = None;
        let mut escaped = false;
        let mut start = 0;
        for (i, &c) in chars.iter().enumerate().take(cursor) {
            if escaped {
                word.push(c);
                escaped = false;
                continue;
            }
            if c == '\\' && quote != Some('\'') {
                escaped = true;
                continue;
            }
            if Some(c) == quote {
                quote = None;
                continue;
            }
            if quote.is_none() && matches!(c, '\'' | '"') {
                quote = Some(c);
                continue;
            }
            if quote.is_none() && c.is_whitespace() {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                start = i + 1;
            } else {
                word.push(c);
            }
        }
        let mut end = cursor;
        let mut tail_quote = quote;
        let mut esc = false;
        for &c in &chars[cursor..] {
            if !esc && tail_quote.is_none() && c.is_whitespace() {
                break;
            }
            end += 1;
            if esc {
                esc = false;
            } else if c == '\\' && tail_quote != Some('\'') {
                esc = true;
            } else if Some(c) == tail_quote {
                tail_quote = None;
            } else if tail_quote.is_none() && matches!(c, '\'' | '"') {
                tail_quote = Some(c);
            }
        }
        words.push(word.clone());
        Self {
            words,
            word,
            start,
            end,
            cursor,
            quote,
        }
    }
    pub fn cache_key(&self, buffer: &str, cwd: &str) -> String {
        let before: String = buffer.chars().take(self.start).collect();
        let dir = self.word.rsplit_once('/').map(|(a, _)| a).unwrap_or("");
        format!("{cwd}\x1e{before}\x1e{dir}")
    }
}
pub fn rank(
    candidates: &[Candidate],
    query: &Query,
    history: &[Entry],
    usage: &HashMap<String, f64>,
) -> Vec<Candidate> {
    use nucleo_matcher::{
        Config as MatchConfig, Matcher, Utf32Str,
        pattern::{AtomKind, CaseMatching, Normalization, Pattern},
    };
    thread_local! {static MATCHER:std::cell::RefCell<Matcher>=std::cell::RefCell::new(Matcher::new(MatchConfig::DEFAULT));}
    MATCHER.with(|cell| {
        let mut matcher = cell.borrow_mut();
        let mut buf = vec![];
        let pattern = Pattern::new(
            &query.word,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut groups: HashMap<&str, usize> = HashMap::new();
        let mut results = vec![];
        for candidate in candidates {
            let composed;
            let searchable = if candidate.prefix.is_empty() {
                candidate.word.as_str()
            } else {
                composed = format!("{}{}", candidate.prefix, candidate.word);
                &composed
            };
            let score = if query.word.is_empty() {
                Some(0)
            } else {
                pattern.score(Utf32Str::new(searchable, &mut buf), &mut matcher)
            };
            // Clone only survivors: most of a large candidate list is
            // filtered out by the fuzzy pattern.
            if let Some(score) = score {
                let next = groups.len();
                let group = *groups.entry(&candidate.group).or_insert(next);
                let mut c = candidate.clone();
                let exact = !query.word.is_empty()
                    && (searchable == query.word
                        || searchable.strip_suffix('/') == Some(query.word.as_str()));
                c.score = f64::from(score)
                    + if exact {
                        20000.
                    } else if searchable.starts_with(&query.word) {
                        10000.
                    } else {
                        0.
                    }
                    + usage.get(&c.word).copied().unwrap_or(0.) * 100.;
                c.matches.clear();
                results.push((group, c));
            }
        }
        results.sort_by(|(ga, a), (gb, b)| ga.cmp(gb).then_with(|| b.score.total_cmp(&a.score)));
        // compsys can offer one insertion from several groups (e.g. the
        // `echo` builtin and /usr/bin/echo). Keep the first, best-ranked one.
        let mut seen = std::collections::HashSet::new();
        let mut ranked: Vec<_> = results
            .into_iter()
            .map(|(_, c)| c)
            .filter(|c| seen.insert((c.prefix.clone(), c.word.clone(), c.suffix.clone())))
            .collect();
        ranked.extend(
            history
                .iter()
                .take(2)
                .map(|e| Candidate::history(e.command.clone(), e.score)),
        );
        ranked
    })
}
pub fn match_positions(word: &str, prefix: &str, needle: &str) -> Vec<u32> {
    use nucleo_matcher::{
        Config, Matcher, Utf32Str,
        pattern::{AtomKind, CaseMatching, Normalization, Pattern},
    };
    thread_local! {static MATCHER:std::cell::RefCell<Matcher>=std::cell::RefCell::new(Matcher::new(Config::DEFAULT));}
    if needle.is_empty() {
        return vec![];
    }
    MATCHER.with(|cell| {
        let mut matcher = cell.borrow_mut();
        let mut buf = vec![];
        let searchable = format!("{prefix}{word}");
        let mut indices = vec![];
        Pattern::new(
            needle,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
        )
        .indices(
            Utf32Str::new(&searchable, &mut buf),
            &mut matcher,
            &mut indices,
        );
        let shift = prefix.chars().count() as u32;
        indices
            .into_iter()
            .filter_map(|i| i.checked_sub(shift))
            .collect()
    })
}
pub fn ghost<'a>(buffer: &str, cursor: usize, history: &'a [Entry]) -> Option<&'a str> {
    if buffer.is_empty() || cursor != buffer.chars().count() {
        return None;
    }
    history.iter().find_map(|h| {
        h.command
            .strip_prefix(buffer)
            .filter(|s| !s.is_empty() && !s.chars().any(|c| c.is_control() && c != '\n'))
    })
}
pub fn should_show(
    buffer: &str,
    query: &Query,
    keymap: &str,
    flags: &str,
    candidates: &[Candidate],
    config: &Config,
) -> bool {
    !buffer.is_empty()
        && !flags.contains('h')
        && keymap != "vicmd"
        && !query.word.contains(['*', '?', '['])
        && (query.words.len() > 1
            || query.word.chars().count() >= config.menu.min_prefix_first_word)
        && !candidates.is_empty()
        && !(candidates.len() == 1
            && candidates[0].kind != Kind::History
            && candidates[0].word == query.word)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_quotes_and_unicode() {
        let q = Query::parse("cat 'My Docs' x", 10);
        assert_eq!(q.word, "My Do");
        assert_eq!(q.start, 4);
        assert_eq!(q.end, 13);
        let q = Query::parse("echo файл", 9);
        assert_eq!(q.word, "файл");
        let q = Query::parse("git ", 4);
        assert_eq!(q.words, vec!["git", ""]);
    }
    #[test]
    fn prefix_and_stability() {
        let mut a = Candidate::history("cherry".into(), 0.);
        a.kind = Kind::Argument;
        let mut b = a.clone();
        b.word = "checkout".into();
        let mut c = a.clone();
        c.word = "catch".into();
        let q = Query::parse("ch", 2);
        let out = rank(&[a, b, c], &q, &[], &HashMap::new());
        assert!(out[0].word.starts_with("ch"));
        assert_eq!(out.last().unwrap().word, "catch");
    }
    #[test]
    fn same_insertion_from_two_groups_is_shown_once() {
        let mut builtin = Candidate::history("echo".into(), 0.);
        builtin.group = "builtin".into();
        builtin.kind = Kind::Builtin;
        let mut binary = builtin.clone();
        binary.group = "command".into();
        binary.kind = Kind::Command;
        let out = rank(&[builtin, binary], &Query::parse("ec", 2), &[], &HashMap::new());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, Kind::Builtin);
    }
    #[test]
    fn exact_match_comes_first() {
        let words = ["gsf", "gsx", "gs", "gsettings"];
        let candidates: Vec<_> = words
            .iter()
            .map(|w| Candidate::history((*w).into(), 0.))
            .collect();
        let out = rank(&candidates, &Query::parse("gs", 2), &[], &HashMap::new());
        assert_eq!(out[0].word, "gs");
    }
    #[test]
    fn ghost_requires_end() {
        let h = vec![Entry {
            command: "echo hello".into(),
            score: 1.,
        }];
        assert_eq!(ghost("echo", 4, &h), Some(" hello"));
        assert_eq!(ghost("echo", 2, &h), None);
    }
}
#[cfg(test)]
mod behavior_tests {
    use super::*;
    fn candidate(word: &str) -> Candidate {
        let mut c = Candidate::history(word.into(), 0.);
        c.kind = Kind::Argument;
        c
    }
    #[test]
    fn history_boost_and_stable_ties() {
        let candidates = vec![candidate("feature-a"), candidate("feature-b")];
        let q = Query::parse("f", 1);
        let plain = rank(&candidates, &q, &[], &HashMap::new());
        assert_eq!(plain[0].word, "feature-a");
        let boosted = rank(
            &candidates,
            &q,
            &[],
            &HashMap::from([("feature-b".into(), 10.)]),
        );
        assert_eq!(boosted[0].word, "feature-b");
        let history = vec![
            Entry {
                command: "foo".into(),
                score: 1.
            };
            10
        ];
        assert_eq!(rank(&[], &q, &history, &HashMap::new()).len(), 2);
    }
    #[test]
    fn visibility_rules() {
        let c = Config::default();
        let candidates = vec![candidate("checkout")];
        for (buffer, keymap, flags, want) in [
            ("", "main", "", false),
            ("g", "main", "", true),
            ("git ", "main", "", true),
            ("git c", "main", "h", false),
            ("git *", "main", "", false),
            ("git ?", "main", "", false),
            ("git [", "main", "", false),
            ("git c", "vicmd", "", false),
            ("checkout", "main", "", false),
        ] {
            let q = Query::parse(buffer, buffer.chars().count());
            assert_eq!(
                should_show(buffer, &q, keymap, flags, &candidates, &c),
                want,
                "{buffer}"
            );
        }
    }
    #[test]
    fn path_prefix_matching() {
        let mut c = candidate("Desktop/");
        c.prefix = "~/".into();
        let q = Query::parse("ls ~/De", 7);
        assert_eq!(rank(&[c], &q, &[], &HashMap::new()).len(), 1);
    }
}

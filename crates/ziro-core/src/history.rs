use rusqlite::{Connection, params};
use std::{collections::HashMap, path::Path};
#[derive(Debug, Clone)]
pub struct Entry {
    pub command: String,
    pub score: f64,
}
pub struct HistoryStore {
    db: Connection,
    pub cwd_boost: f64,
    pub failed_penalty: f64,
}
impl HistoryStore {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }
    pub fn memory() -> rusqlite::Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }
    fn from_connection(db: Connection) -> rusqlite::Result<Self> {
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS history(id TEXT PRIMARY KEY, ts REAL NOT NULL, cwd TEXT NOT NULL, command TEXT NOT NULL, exit INTEGER, duration_ms INTEGER); CREATE INDEX IF NOT EXISTS history_command ON history(command); CREATE TABLE IF NOT EXISTS imports(path TEXT PRIMARY KEY, count INTEGER NOT NULL);")?;
        Ok(Self {
            db,
            cwd_boost: 1.5,
            failed_penalty: 0.3,
        })
    }
    pub fn record_start(
        &self,
        id: &str,
        ts: f64,
        cwd: &str,
        command: &str,
    ) -> rusqlite::Result<()> {
        self.db.execute(
            "INSERT OR IGNORE INTO history(id,ts,cwd,command) VALUES(?1,?2,?3,?4)",
            params![id, ts, cwd, command],
        )?;
        Ok(())
    }
    pub fn record_end(&self, id: &str, exit: i32, duration_ms: u64) -> rusqlite::Result<()> {
        self.db.execute(
            "UPDATE history SET exit=?2,duration_ms=?3 WHERE id=?1",
            params![id, exit, duration_ms.min(i64::MAX as u64) as i64],
        )?;
        Ok(())
    }
    pub fn len(&self) -> rusqlite::Result<u64> {
        self.db.query_row("SELECT count(*) FROM history", [], |r| {
            r.get::<_, i64>(0).map(|n| n as u64)
        })
    }
    pub fn is_empty(&self) -> rusqlite::Result<bool> {
        Ok(self.len()? == 0)
    }
    pub fn import_histfile(&mut self, path: &Path) -> Result<usize, Box<dyn std::error::Error>> {
        let bytes = std::fs::read(path)?;
        let mut raw = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x83 && i + 1 < bytes.len() {
                i += 1;
                raw.push(bytes[i] ^ 32);
            } else {
                raw.push(bytes[i]);
            }
            i += 1;
        }
        let text = String::from_utf8_lossy(&raw);
        let mut records = vec![];
        let mut current = String::new();
        for line in text.lines() {
            current.push_str(line);
            if current.ends_with('\\') {
                current.pop();
                current.push('\n');
            } else {
                records.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            records.push(current);
        }
        let tx = self.db.transaction()?;
        let mut added = 0;
        // Occurrence-based identity preserves genuine repeated commands, and repeated imports.
        let path = path.to_string_lossy();
        let mut occurrences: HashMap<String, usize> = HashMap::new();
        for record in records {
            let (ts, command) = if let Some(rest) = record.strip_prefix(": ") {
                if let Some((meta, cmd)) = rest.split_once(';') {
                    (
                        meta.split(':')
                            .next()
                            .unwrap_or("0")
                            .parse::<f64>()
                            .unwrap_or(0.),
                        cmd,
                    )
                } else {
                    (0., record.as_str())
                }
            } else {
                (0., record.as_str())
            };
            if command.is_empty() {
                continue;
            }
            let occurrence = occurrences.entry(record.clone()).or_default();
            *occurrence += 1;
            let id = format!("import:{path}:{record}:{}", *occurrence);
            added += tx.execute(
                "INSERT OR IGNORE INTO history(id,ts,cwd,command,exit) VALUES(?1,?2,'',?3,0)",
                params![id, ts, command],
            )?;
        }
        tx.commit()?;
        Ok(added)
    }
    pub fn prefix_matches(
        &self,
        prefix: &str,
        cwd: &str,
        now: f64,
        limit: usize,
    ) -> rusqlite::Result<Vec<Entry>> {
        let mut stmt = self
            .db
            .prepare("SELECT command,ts,cwd,exit FROM history WHERE command>=?1 AND command<?2")?;
        let upper = format!("{prefix}\u{10ffff}");
        let rows = stmt.query_map(params![prefix, upper], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i32>>(3)?,
            ))
        })?;
        let mut scores: HashMap<String, f64> = HashMap::new();
        for row in rows {
            let (cmd, ts, dir, exit) = row?;
            let age = (now - ts).max(0.);
            let weight = (-age / 604800.).exp()
                * if dir == cwd { self.cwd_boost } else { 1. }
                * if exit.is_some_and(|x| x != 0) {
                    self.failed_penalty
                } else {
                    1.
                };
            *scores.entry(cmd).or_default() += weight;
        }
        let mut result: Vec<_> = scores
            .into_iter()
            .map(|(command, score)| Entry { command, score })
            .collect();
        result.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.command.cmp(&b.command))
        });
        result.truncate(limit);
        Ok(result)
    }
    pub fn arg_usage(
        &self,
        words: &[String],
        cwd: &str,
        now: f64,
    ) -> rusqlite::Result<HashMap<String, f64>> {
        let Some(command) = words.first() else {
            return Ok(HashMap::new());
        };
        let entries = self.prefix_matches(&format!("{command} "), cwd, now, 10000)?;
        let position = words.len().saturating_sub(1);
        let mut usage = HashMap::new();
        for entry in entries {
            let q = crate::rank::Query::parse(&entry.command, entry.command.chars().count());
            if let Some(arg) = q.words.get(position) {
                *usage.entry(arg.clone()).or_default() += entry.score;
            }
        }
        Ok(usage)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weights_and_position() {
        let h = HistoryStore::memory().unwrap();
        h.record_start("1", 100., "/a", "git checkout main")
            .unwrap();
        h.record_end("1", 0, 5).unwrap();
        h.record_start("2", 100., "/b", "git checkout failed")
            .unwrap();
        h.record_end("2", 1, 5).unwrap();
        let v = h.prefix_matches("git", "/a", 100., 10).unwrap();
        assert_eq!(v[0].score, 1.5);
        assert_eq!(v[1].score, 0.3);
        assert_eq!(
            h.arg_usage(&["git".into(), "checkout".into(), "m".into()], "/a", 100.)
                .unwrap()["main"],
            1.5
        );
    }
    #[test]
    fn imports() {
        let path = std::env::temp_dir().join(format!("ziro-history-{}", std::process::id()));
        std::fs::write(
            &path,
            b": 100:0;echo hi\necho one\\\ntwo\necho \x83\xf0\x83\x9f\n",
        )
        .unwrap();
        let mut h = HistoryStore::memory().unwrap();
        assert_eq!(h.import_histfile(&path).unwrap(), 3);
        assert_eq!(h.import_histfile(&path).unwrap(), 0);
        assert!(
            h.prefix_matches("echo one\ntwo", "", 100., 10)
                .unwrap()
                .len()
                == 1
        );
        std::fs::remove_file(path).unwrap();
    }
}

use rusqlite::{Connection, params};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};
#[derive(Debug, Clone)]
pub struct Entry {
    pub command: String,
    pub score: f64,
}
/// Decay half-scale of history weights: one week.
const DECAY_SECONDS: f64 = 604800.;
/// Aggregated weights of one distinct command line.
///
/// A row's weight is `exp(-(now - ts) / W)`, which factors into
/// `exp(-(now - t0) / W) * exp((ts - t0) / W)`: sums of the second factor are
/// kept per command, so a lookup never revisits individual rows.
#[derive(Default)]
struct Aggregate {
    words: Vec<String>,
    ok: f64,
    failed: f64,
    /// (cwd id, ok, failed)
    by_cwd: Vec<(u32, f64, f64)>,
}
impl Aggregate {
    fn add(&mut self, cwd: u32, weight: f64, failed: bool) {
        let slot = match self.by_cwd.iter_mut().position(|(c, _, _)| *c == cwd) {
            Some(i) => &mut self.by_cwd[i],
            None => {
                self.by_cwd.push((cwd, 0., 0.));
                self.by_cwd.last_mut().unwrap()
            }
        };
        if failed {
            self.failed += weight;
            slot.2 += weight;
        } else {
            self.ok += weight;
            slot.1 += weight;
        }
    }
}
pub struct HistoryStore {
    db: Connection,
    pub cwd_boost: f64,
    pub failed_penalty: f64,
    index: BTreeMap<String, Aggregate>,
    cwds: HashMap<String, u32>,
    /// Commands started but not finished: id -> (command, ts, cwd id).
    running: HashMap<String, (String, f64, u32)>,
    t0: Option<f64>,
    rows: u64,
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
        let mut store = Self {
            db,
            cwd_boost: 1.5,
            failed_penalty: 0.3,
            index: BTreeMap::new(),
            cwds: HashMap::new(),
            running: HashMap::new(),
            t0: None,
            rows: 0,
        };
        store.reload()?;
        Ok(store)
    }
    fn reload(&mut self) -> rusqlite::Result<()> {
        let rows = {
            let mut stmt = self.db.prepare("SELECT command,ts,cwd,exit FROM history")?;
            stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i32>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        self.index.clear();
        self.rows = rows.len() as u64;
        self.t0 = rows.iter().map(|r| r.1).reduce(f64::max);
        for (command, ts, cwd, exit) in rows {
            self.add(command, ts, &cwd, exit.is_some_and(|x| x != 0));
        }
        Ok(())
    }
    fn cwd_id(&mut self, cwd: &str) -> u32 {
        let next = self.cwds.len() as u32;
        *self.cwds.entry(cwd.to_owned()).or_insert(next)
    }
    fn weight(&mut self, ts: f64) -> f64 {
        let t0 = *self.t0.get_or_insert(ts);
        ((ts - t0) / DECAY_SECONDS).exp()
    }
    fn add(&mut self, command: String, ts: f64, cwd: &str, failed: bool) {
        let cwd = self.cwd_id(cwd);
        let weight = self.weight(ts);
        let entry = self.index.entry(command).or_insert_with_key(|c| Aggregate {
            words: crate::rank::Query::parse(c, c.chars().count()).words,
            ..Aggregate::default()
        });
        entry.add(cwd, weight, failed);
    }
    pub fn record_start(
        &mut self,
        id: &str,
        ts: f64,
        cwd: &str,
        command: &str,
    ) -> rusqlite::Result<()> {
        let inserted = self.db.execute(
            "INSERT OR IGNORE INTO history(id,ts,cwd,command) VALUES(?1,?2,?3,?4)",
            params![id, ts, cwd, command],
        )?;
        if inserted > 0 {
            self.rows += 1;
            self.add(command.to_owned(), ts, cwd, false);
            let cwd = self.cwd_id(cwd);
            self.running
                .insert(id.to_owned(), (command.to_owned(), ts, cwd));
        }
        Ok(())
    }
    pub fn record_end(&mut self, id: &str, exit: i32, duration_ms: u64) -> rusqlite::Result<()> {
        let updated = self.db.execute(
            "UPDATE history SET exit=?2,duration_ms=?3 WHERE id=?1 AND exit IS NULL",
            params![id, exit, duration_ms.min(i64::MAX as u64) as i64],
        )?;
        let started = match self.running.remove(id) {
            Some(started) => Some(started),
            // Started before this daemon: the row was loaded as unfinished.
            None if updated > 0 => self
                .db
                .query_row(
                    "SELECT command,ts,cwd FROM history WHERE id=?1",
                    params![id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, f64>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )
                .ok()
                .map(|(c, ts, cwd)| {
                    let cwd = self.cwd_id(&cwd);
                    (c, ts, cwd)
                }),
            None => None,
        };
        if exit != 0
            && updated > 0
            && let Some((command, ts, cwd)) = started
        {
            // Move the run from the successful to the failed weights.
            let weight = self.weight(ts);
            if let Some(entry) = self.index.get_mut(&command) {
                entry.add(cwd, -weight, false);
                entry.add(cwd, weight, true);
            }
        }
        Ok(())
    }
    fn score(&self, a: &Aggregate, cwd: Option<u32>, decay: f64) -> f64 {
        let mut score = a.ok + a.failed * self.failed_penalty;
        if let Some(cwd) = cwd
            && let Some((_, ok, failed)) = a.by_cwd.iter().find(|(c, _, _)| *c == cwd)
        {
            score += (self.cwd_boost - 1.) * (ok + failed * self.failed_penalty);
        }
        score * decay
    }
    fn decay(&self, now: f64) -> f64 {
        self.t0
            .map(|t0| (-(now - t0) / DECAY_SECONDS).exp())
            .unwrap_or(1.)
    }
    fn matching<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = (&'a String, &'a Aggregate)> {
        self.index
            .range::<str, _>((
                std::ops::Bound::Included(prefix),
                std::ops::Bound::Unbounded,
            ))
            .take_while(move |(c, _)| c.starts_with(prefix))
    }
    pub fn len(&self) -> rusqlite::Result<u64> {
        Ok(self.rows)
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
        if added > 0 {
            self.reload()?;
        }
        Ok(added)
    }
    pub fn prefix_matches(
        &self,
        prefix: &str,
        cwd: &str,
        now: f64,
        limit: usize,
    ) -> rusqlite::Result<Vec<Entry>> {
        let cwd = self.cwds.get(cwd).copied();
        let decay = self.decay(now);
        let mut scored: Vec<(f64, &String)> = self
            .matching(prefix)
            .map(|(command, a)| (self.score(a, cwd, decay), command))
            .collect();
        let order =
            |a: &(f64, &String), b: &(f64, &String)| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1));
        if limit == 0 {
            return Ok(vec![]);
        }
        if scored.len() > limit {
            scored.select_nth_unstable_by(limit - 1, order);
            scored.truncate(limit);
        }
        scored.sort_by(order);
        Ok(scored
            .into_iter()
            .map(|(score, command)| Entry {
                command: command.clone(),
                score,
            })
            .collect())
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
        let position = words.len().saturating_sub(1);
        let mut usage = HashMap::new();
        let cwd = self.cwds.get(cwd).copied();
        let decay = self.decay(now);
        let prefix = format!("{command} ");
        for (_, a) in self.matching(&prefix) {
            if let Some(arg) = a.words.get(position) {
                *usage.entry(arg.clone()).or_default() += self.score(a, cwd, decay);
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
        let mut h = HistoryStore::memory().unwrap();
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
    fn index_matches_row_formula_and_survives_restart() {
        let path = std::env::temp_dir().join(format!("ziro-hidx-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut rows = vec![];
        {
            let mut h = HistoryStore::open(&path).unwrap();
            for i in 0..200u32 {
                let cmd = format!(
                    "git {} x{}",
                    ["checkout", "push", "pull"][i as usize % 3],
                    i % 7
                );
                let cwd = ["/a", "/b", "/c"][i as usize % 3 % 2];
                let ts = 1_000_000. + f64::from(i) * 3600.;
                let id = i.to_string();
                h.record_start(&id, ts, cwd, &cmd).unwrap();
                // Leave some runs unfinished; they finish after a restart.
                let exit = if i % 4 == 0 { 1 } else { 0 };
                if i % 10 != 0 {
                    h.record_end(&id, exit, 1).unwrap();
                }
                rows.push((cmd, ts, cwd, exit));
            }
        }
        let mut h = HistoryStore::open(&path).unwrap();
        for (i, row) in rows.iter().enumerate().filter(|(i, _)| i % 10 == 0) {
            h.record_end(&i.to_string(), row.3, 1).unwrap();
        }
        let now = 1_000_000. + 250. * 3600.;
        let brute = |prefix: &str, cwd: &str| {
            let mut m: HashMap<String, f64> = HashMap::new();
            for (cmd, ts, dir, exit) in &rows {
                if cmd.starts_with(prefix) {
                    *m.entry(cmd.clone()).or_default() += (-(now - ts) / DECAY_SECONDS).exp()
                        * if *dir == cwd { h.cwd_boost } else { 1. }
                        * if *exit != 0 { h.failed_penalty } else { 1. };
                }
            }
            m
        };
        for (prefix, cwd) in [("git p", "/a"), ("git", "/b"), ("git checkout x3", "/zzz")] {
            let expected = brute(prefix, cwd);
            let got = h.prefix_matches(prefix, cwd, now, 1000).unwrap();
            assert_eq!(got.len(), expected.len());
            for e in got {
                assert!(
                    (e.score - expected[&e.command]).abs() < 1e-9,
                    "{}",
                    e.command
                );
            }
        }
        std::fs::remove_file(&path).ok();
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
#[cfg(test)]
mod bench {
    use super::*;
    #[test]
    #[ignore]
    fn large_history_latency() {
        let mut h = HistoryStore::memory().unwrap();
        let cmds = [
            "git checkout feature-",
            "git status",
            "cargo build --release",
            "cd ~/Desktop/proj",
            "ls -la /tmp/",
            "grep -rn foo src/",
            "docker compose up -d svc",
        ];
        h.db.execute_batch("BEGIN").unwrap();
        for i in 0..100_000 {
            let c = format!("{}{}", cmds[i % cmds.len()], i % 3000);
            h.record_start(
                &i.to_string(),
                1_700_000_000. + i as f64,
                if i % 5 == 0 { "/a" } else { "/b" },
                &c,
            )
            .unwrap();
            h.record_end(&i.to_string(), 0, 1).unwrap();
        }
        h.db.execute_batch("COMMIT").unwrap();
        for prefix in ["g", "git ", "git checkout f", "c"] {
            let t = std::time::Instant::now();
            let n = h
                .prefix_matches(prefix, "/a", 1_700_100_000., 8)
                .unwrap()
                .len();
            println!("prefix_matches({prefix:?}) -> {n} in {:?}", t.elapsed());
        }
        for words in [vec!["git", "checkout", ""], vec!["cd", ""], vec!["g"]] {
            let words: Vec<String> = words.into_iter().map(String::from).collect();
            let t = std::time::Instant::now();
            let n = h.arg_usage(&words, "/a", 1_700_100_000.).unwrap().len();
            println!("arg_usage({words:?}) -> {n} in {:?}", t.elapsed());
        }
    }
}

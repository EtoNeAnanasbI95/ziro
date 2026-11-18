use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{Mutex, mpsc},
};
use ziro_core::{
    candidate::{Candidate, Context},
    config::Config,
    history::HistoryStore,
    layout,
    rank::{self, Query},
};
use ziro_proto::{Framer, Message};
type Tx = mpsc::UnboundedSender<Message>;
#[derive(Default)]
struct Session {
    shell: Option<Tx>,
    worker: Option<Tx>,
    generation: u64,
    buffer: String,
    cursor: usize,
    cols: usize,
    /// Screen column where the edit buffer starts (prompt width), if known.
    origin: Option<usize>,
    keymap: String,
    cwd: String,
    worker_cwd: String,
    worker_failed: bool,
    flags: String,
    cache_key: String,
    context: Context,
    candidates: Vec<Candidate>,
    ranked: Vec<Candidate>,
    selected: usize,
    busy: Option<(u64, Instant)>,
    pending: bool,
    batch: Vec<Candidate>,
    batch_context: Context,
    batch_gen: u64,
    batch_count: usize,
    last_edit: Option<Instant>,
    /// Buffer/cursor of the in-flight request and of the current candidates.
    requested: Option<(String, usize)>,
    answered: Option<(String, usize)>,
    /// Recent completed contexts, shown immediately while compsys refreshes.
    recent: std::collections::VecDeque<(String, Context, Vec<Candidate>)>,
}
const RECENT_CONTEXTS: usize = 16;
const RECENT_MAX_CANDIDATES: usize = 2000;
struct State {
    sessions: HashMap<String, Session>,
    history: HistoryStore,
    config: Config,
    ctx_us: Vec<u64>,
    trace_us: Vec<u64>,
    /// Stats changed since the last write of stats.json.
    stats_dirty: bool,
    /// Wakes the timer task when a session starts waiting on something.
    wake: Rc<tokio::sync::Notify>,
}
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("ziro")
}
pub fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/ziro-{uid}")))
        .join(if std::env::var_os("XDG_RUNTIME_DIR").is_some() {
            "ziro/daemon.sock"
        } else {
            "daemon.sock"
        })
}
pub fn config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
        .join("ziro/config.toml")
}
fn configuration(c: &Config) -> Message {
    let mut fields = vec![
        c.keys.arrows.clone(),
        c.menu.enter_accepts.clone(),
        "32".into(),
        c.keys.accept.clone(),
        c.keys.enter_menu.clone(),
        c.keys.dismiss.clone(),
    ];
    fields.extend(c.keys.next.clone());
    fields.push("|".into());
    fields.extend(c.keys.prev.clone());
    Message::new("CFG", fields)
}
fn send(tx: &Option<Tx>, m: Message) {
    if let Some(tx) = tx {
        let _ = tx.send(m);
    }
}
fn request(s: &mut Session) {
    if s.buffer.is_empty()
        || s.keymap == "vicmd"
        || s.flags.contains('h')
        || Query::parse(&s.buffer, s.cursor)
            .word
            .contains(['*', '?', '['])
    {
        s.pending = false;
        return;
    }
    if s.busy.is_some() {
        s.pending = true;
        return;
    }
    if s.worker.is_none() {
        s.pending = true;
        return;
    }
    send(
        &s.worker,
        Message::new(
            "REQ",
            [
                s.generation.to_string(),
                s.cursor.to_string(),
                s.buffer.clone(),
            ],
        ),
    );
    s.busy = Some((s.generation, Instant::now()));
    s.requested = Some((s.buffer.clone(), s.cursor));
    s.pending = false;
}
fn frame(s: &mut Session, h: &HistoryStore, c: &Config, waiting: bool) {
    let q = Query::parse(&s.buffer, s.cursor);
    let history = h
        .prefix_matches(&s.buffer, &s.cwd, now(), 8)
        .unwrap_or_default();
    let usage = h.arg_usage(&q.words, &s.cwd, now()).unwrap_or_default();
    let mut matching_query = q.clone();
    if let Some(word) = matching_query.word.strip_prefix(&s.context.iprefix) {
        matching_query.word = word.to_owned();
    }
    s.ranked = rank::rank(&s.candidates, &matching_query, &history, &usage);
    let visible = rank::should_show(&s.buffer, &q, &s.keymap, &s.flags, &s.ranked, c);
    let ghost = if c.ghost.enable && !s.flags.contains('h') && s.keymap != "vicmd" {
        rank::ghost(&s.buffer, s.cursor, &history).unwrap_or("")
    } else {
        ""
    };
    if !visible && ghost.is_empty() {
        let can_complete = !s.buffer.is_empty()
            && s.keymap != "vicmd"
            && !s.flags.contains('h')
            && !q.word.contains(['*', '?', '[']);
        send(
            &s.shell,
            Message::new(
                if waiting && can_complete {
                    "WAIT"
                } else {
                    "CLEAR"
                },
                [s.generation],
            ),
        );
        return;
    }
    let f = layout::layout_at(
        s.generation,
        &s.buffer,
        &q,
        &s.context,
        if visible { &s.ranked } else { &[] },
        s.selected,
        ghost,
        s.cols,
        s.origin,
        c,
    );
    send(&s.shell, f.message());
}
fn stats(st: &State) -> serde_json::Value {
    fn percentiles(values: &[u64]) -> serde_json::Value {
        let mut v = values.to_vec();
        v.sort_unstable();
        let at = |p: usize| {
            v.get(v.len().saturating_sub(1) * p / 100)
                .copied()
                .unwrap_or(0)
        };
        serde_json::json!({"count":v.len(),"p50_us":at(50),"p95_us":at(95),"p99_us":at(99)})
    }
    serde_json::json!({"history_entries":st.history.len().unwrap_or(0),"ctx_to_frame":percentiles(&st.ctx_us),"plugin":percentiles(&st.trace_us),"sessions":st.sessions.len()})
}
fn persist_stats(st: &State) {
    if let Ok(bytes) = serde_json::to_vec_pretty(&stats(st)) {
        let _ = fs::write(state_dir().join("stats.json"), bytes);
    }
}
async fn client(stream: UnixStream, state: Rc<Mutex<State>>) {
    let (mut reader, mut writer) = stream.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let writer_task = tokio::task::spawn_local(async move {
        while let Some(msg) = rx.recv().await {
            match msg.encode() {
                Ok(bytes) => {
                    if writer.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                Err(e) => tracing::warn!(%e,"encode"),
            }
        }
    });
    let mut framer = Framer::default();
    let mut bytes = [0; 65536];
    let mut session_id = String::new();
    let mut worker = false;
    'connection: loop {
        let n = match reader.read(&mut bytes).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let messages = match framer.push(&bytes[..n]) {
            Ok(v) => v,
            Err(_) => break,
        };
        for m in messages {
            let started = Instant::now();
            let mut st = state.lock().await;
            match m.kind.as_str() {
                "HELLO" => {
                    if m.field(0) != "v1" {
                        let _ = tx.send(Message::new("ERR", ["proto", "expected v1"]));
                        break 'connection;
                    }
                    session_id = m.field(1).into();
                    let _ = tx.send(configuration(&st.config));
                    st.sessions.entry(session_id.clone()).or_default().shell = Some(tx.clone());
                }
                "WORKER_HELLO" => {
                    session_id = m.field(0).into();
                    worker = true;
                    if let Some(s) = st.sessions.get_mut(&session_id) {
                        s.worker_failed = false;
                        s.worker_cwd = m.field(1).into();
                        s.worker = Some(tx.clone());
                        if s.pending {
                            request(s);
                            st.wake.notify_one();
                        }
                    } else {
                        break 'connection;
                    }
                }
                "HIST_START" if !worker && !session_id.is_empty() => {
                    let _ = st.history.record_start(
                        m.field(0),
                        m.number(1).unwrap_or(now()),
                        m.field(2),
                        m.field(3),
                    );
                }
                "HIST_END" if !worker && !session_id.is_empty() => {
                    let _ = st.history.record_end(
                        m.field(0),
                        m.number(1).unwrap_or(0),
                        m.number(2).unwrap_or(0),
                    );
                }
                "TRACE" if !worker => {
                    for x in m.fields.iter().skip(1) {
                        if let Ok(x) = x.parse() {
                            st.trace_us.push(x);
                            st.stats_dirty = true;
                        }
                    }
                    let excess = st.trace_us.len().saturating_sub(10000);
                    st.trace_us.drain(..excess);
                }
                "BYE" => break 'connection,
                _ => {
                    // Remove while mutating so history and config remain separately borrowable.
                    let Some(mut s) = st.sessions.remove(&session_id) else {
                        break 'connection;
                    };
                    match m.kind.as_str() {
                        "CTX" if !worker => {
                            let Ok(generation) = m.number::<u64>(0) else {
                                break 'connection;
                            };
                            s.generation = generation;
                            s.cursor = m.number(1).unwrap_or(0);
                            s.cols = m.number::<usize>(2).unwrap_or(80).clamp(5, 1000);
                            s.keymap = m.field(4).into();
                            s.cwd = m.field(5).into();
                            if s.worker.is_some() && s.worker_cwd != s.cwd {
                                s.worker = None;
                                s.busy = None;
                                s.pending = true;
                                send(
                                    &s.shell,
                                    Message::new(
                                        "ERR",
                                        ["worker_cwd", "recreate worker in current directory"],
                                    ),
                                );
                            }
                            s.flags = m.field(6).into();
                            s.buffer = m.field(7).into();
                            s.origin = m.field(8).parse().ok();
                            if s.worker_failed
                                && !s.buffer.is_empty()
                                && !s.flags.contains('h')
                                && s.keymap != "vicmd"
                            {
                                s.worker_failed = false;
                                send(
                                    &s.shell,
                                    Message::new(
                                        "ERR",
                                        [
                                            "worker_restart",
                                            "restart completion worker after failure",
                                        ],
                                    ),
                                );
                            }
                            s.selected = 0;
                            let q = Query::parse(&s.buffer, s.cursor);
                            let key = q.cache_key(&s.buffer, &s.cwd);
                            let changed = key != s.cache_key;
                            if changed {
                                s.candidates.clear();
                                s.answered = None;
                                // Stale-while-revalidate: reuse the last answer
                                // for this context until the worker replies.
                                if let Some((_, context, candidates)) =
                                    s.recent.iter().find(|(k, _, _)| *k == key)
                                {
                                    s.context = context.clone();
                                    s.candidates = candidates.clone();
                                }
                                s.cache_key = key;
                            }
                            frame(&mut s, &st.history, &st.config, true);
                            if changed || s.candidates.is_empty() {
                                request(&mut s);
                            }
                            s.last_edit = Some(Instant::now());
                            st.ctx_us.push(started.elapsed().as_micros() as u64);
                            st.stats_dirty = true;
                            st.wake.notify_one();
                            let excess = st.ctx_us.len().saturating_sub(10000);
                            st.ctx_us.drain(..excess);
                        }
                        "CANDS" if worker => {
                            s.batch_gen = m.number(0).unwrap_or(0);
                            s.batch_count = m.number::<usize>(6).unwrap_or(0).min(100000);
                            s.batch.clear();
                            s.batch_context = Context {
                                prefix: m.field(1).into(),
                                suffix: m.field(2).into(),
                                iprefix: m.field(3).into(),
                                isuffix: m.field(4).into(),
                                quote: m.field(5).into(),
                            };
                        }
                        "C" if worker => {
                            if s.batch.len() < s.batch_count {
                                s.batch.push(Candidate::from_record(&m));
                            }
                        }
                        "END" if worker => {
                            let generation = m.number::<u64>(0).unwrap_or(0);
                            if s.busy.is_some_and(|(g, _)| g == generation) {
                                s.busy = None;
                                if generation == s.generation
                                    && s.batch_gen == generation
                                    && s.batch.len() == s.batch_count
                                {
                                    s.candidates = std::mem::take(&mut s.batch);
                                    s.context = s.batch_context.clone();
                                    s.answered = s.requested.take();
                                    if s.candidates.len() <= RECENT_MAX_CANDIDATES {
                                        let key = s.cache_key.clone();
                                        s.recent.retain(|(k, _, _)| *k != key);
                                        s.recent.push_front((
                                            key,
                                            s.context.clone(),
                                            s.candidates.clone(),
                                        ));
                                        s.recent.truncate(RECENT_CONTEXTS);
                                    }
                                    frame(&mut s, &st.history, &st.config, false);
                                }
                                if s.pending {
                                    request(&mut s);
                                }
                            }
                        }
                        "KEY" if !worker && m.number::<u64>(0).ok() == Some(s.generation) => {
                            match m.field(1) {
                                // Tab / Shift+Tab cycle through the list.
                                "next" => {
                                    s.selected = if s.selected + 1 >= s.ranked.len() {
                                        0
                                    } else {
                                        s.selected + 1
                                    }
                                }
                                "prev" => {
                                    s.selected = if s.selected == 0 {
                                        s.ranked.len().saturating_sub(1)
                                    } else {
                                        s.selected - 1
                                    }
                                }
                                "page_down" => {
                                    s.selected = (s.selected + st.config.menu.max_items)
                                        .min(s.ranked.len().saturating_sub(1))
                                }
                                "page_up" => {
                                    s.selected = s.selected.saturating_sub(st.config.menu.max_items)
                                }
                                "dismiss" => {
                                    s.flags.push('h');
                                }
                                _ => {}
                            }
                            frame(&mut s, &st.history, &st.config, false);
                        }
                        _ => {}
                    }
                    st.sessions.insert(session_id.clone(), s);
                }
            }
        }
    }
    let mut st = state.lock().await;
    if worker {
        if let Some(s) = st.sessions.get_mut(&session_id)
            && s.worker
                .as_ref()
                .is_some_and(|active| active.same_channel(&tx))
        {
            s.worker = None;
            s.busy = None;
            s.worker_failed = true;
        }
    } else {
        st.sessions.remove(&session_id);
    }
    persist_stats(&st);
    drop(st);
    drop(tx);
    let _ = writer_task.await;
}
pub async fn serve(path: &Path, config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or("socket needs parent")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let metadata = fs::symlink_metadata(parent)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::getuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(
            "socket directory must be owned by this user, private (0700), and not a symlink".into(),
        );
    }
    let mut lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(parent.join("daemon.lock"))?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("daemon already running".into());
    }
    use std::io::Write;
    lock.set_len(0)?;
    writeln!(lock, "{}", std::process::id())?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::getuid() } {
            return Err("refusing to replace non-socket or another user's socket".into());
        }
        if UnixStream::connect(path).await.is_ok() {
            return Err("socket already active".into());
        }
        fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(state_dir())?;
    let mut history = HistoryStore::open(&state_dir().join("history.db"))?;
    history.cwd_boost = config.history.cwd_boost;
    history.failed_penalty = config.history.failed_penalty;
    if config.history.import_histfile
        && let Some(path) = std::env::var_os("HISTFILE")
        && let Err(e) = history.import_histfile(Path::new(&path))
    {
        tracing::warn!(%e,"history import");
    }
    let state = Rc::new(Mutex::new(State {
        sessions: HashMap::new(),
        history,
        config,
        ctx_us: vec![],
        trace_us: vec![],
        stats_dirty: true,
        wake: Rc::new(tokio::sync::Notify::new()),
    }));
    let reload_state = state.clone();
    let mut hup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;
    tokio::task::spawn_local(async move {
        while hup.recv().await.is_some() {
            match Config::load(&config_path()) {
                Ok((config, warnings)) => {
                    for warning in warnings {
                        tracing::warn!(%warning);
                    }
                    let mut st = reload_state.lock().await;
                    st.history.cwd_boost = config.history.cwd_boost;
                    st.history.failed_penalty = config.history.failed_penalty;
                    for s in st.sessions.values() {
                        send(&s.shell, configuration(&config));
                    }
                    st.config = config;
                }
                Err(e) => tracing::warn!(%e,"keeping previous config"),
            }
        }
    });
    let timer_state = state.clone();
    let wake = state.lock().await.wake.clone();
    tokio::task::spawn_local(async move {
        let mut last_stats = Instant::now();
        let mut waiting = true;
        loop {
            // Poll at 20ms only while a session waits on a worker or an idle
            // refresh; otherwise sleep until an edit arrives.
            if waiting {
                tokio::time::sleep(Duration::from_millis(20)).await;
            } else {
                tokio::select! {
                    _ = wake.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
            }
            let mut st = timer_state.lock().await;
            let timeout = Duration::from_millis(st.config.worker.timeout_ms);
            let idle = Duration::from_millis(st.config.worker.refresh_idle_ms);
            if st.stats_dirty && last_stats.elapsed() > Duration::from_secs(1) {
                persist_stats(&st);
                st.stats_dirty = false;
                last_stats = Instant::now();
            }
            for s in st.sessions.values_mut() {
                if s.busy.is_some_and(|(_, t)| t.elapsed() > timeout) {
                    s.busy = None;
                    s.worker = None;
                    s.worker_failed = true;
                    send(
                        &s.shell,
                        Message::new("ERR", ["worker_timeout", "completion exceeded budget"]),
                    );
                } else if s.busy.is_none() && s.last_edit.is_some_and(|t| t.elapsed() > idle) {
                    s.last_edit = None;
                    // Refresh only candidates computed for an earlier prefix;
                    // re-running compsys for the same line doubles its cost.
                    if s.answered.as_ref() != Some(&(s.buffer.clone(), s.cursor)) {
                        request(s);
                    }
                }
            }
            waiting = st.stats_dirty
                || st
                    .sessions
                    .values()
                    .any(|s| s.busy.is_some() || s.last_edit.is_some());
        }
    });
    tracing::info!(socket=%path.display(),"daemon ready");
    loop {
        let (stream, _) = listener.accept().await?;
        if stream.peer_cred()?.uid() != unsafe { libc::getuid() } {
            continue;
        }
        tokio::task::spawn_local(client(stream, state.clone()));
    }
}

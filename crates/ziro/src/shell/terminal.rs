use std::{
    io::{self, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
pub static RESIZED: AtomicBool = AtomicBool::new(false);
pub static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn signal(signal: libc::c_int) {
    if signal == libc::SIGWINCH {
        RESIZED.store(true, Ordering::Relaxed);
    } else {
        STOP.store(true, Ordering::Relaxed);
    }
}
pub fn size() -> io::Result<libc::winsize> {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(0, libc::TIOCGWINSZ, &mut size) } < 0 {
        return Err(io::Error::last_os_error());
    }
    size.ws_row = size.ws_row.clamp(2, 500);
    size.ws_col = size.ws_col.clamp(5, 1000);
    Ok(size)
}
pub fn nonblock(fd: i32) -> io::Result<()> {
    let old = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if old < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, old | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub struct Terminal {
    pub original: libc::termios,
    flags: i32,
    handlers: Vec<(i32, libc::sigaction)>,
    guardian: Option<Guardian>,
}
impl Terminal {
    pub fn enter() -> io::Result<Self> {
        if unsafe { libc::isatty(0) } != 1 || unsafe { libc::isatty(1) } != 1 {
            return Err(io::Error::other(
                "ziro shell requires an interactive terminal",
            ));
        }
        if unsafe { libc::tcgetpgrp(0) } != unsafe { libc::getpgrp() } {
            return Err(io::Error::other("ziro shell must run in the foreground"));
        }
        let mut original = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(0, &mut original) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let flags = unsafe { libc::fcntl(0, libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        let guardian = Guardian::start(original, flags)?;
        let mut tty = Self {
            original,
            flags,
            handlers: vec![],
            guardian: Some(guardian),
        };
        for sig in [libc::SIGWINCH, libc::SIGTERM, libc::SIGHUP] {
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = signal as extern "C" fn(libc::c_int) as *const () as usize;
            unsafe { libc::sigemptyset(&mut action.sa_mask) };
            let mut old = unsafe { std::mem::zeroed() };
            if unsafe { libc::sigaction(sig, &action, &mut old) } < 0 {
                return Err(io::Error::last_os_error());
            }
            tty.handlers.push((sig, old));
        }
        let mut raw = original;
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } < 0 {
            return Err(io::Error::last_os_error());
        }
        nonblock(0)?;
        Ok(tty)
    }
    pub fn child(&self, pid: u32) {
        if let Some(guardian) = &self.guardian {
            guardian.child(pid);
        }
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        // Normal teardown and unwinding both restore the caller's tty. The
        // guardian handles SIGKILL, where Rust destructors cannot run.
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &self.original);
            libc::fcntl(0, libc::F_SETFL, self.flags);
        }
        let _ = io::stdout().write_all(b"\x1b[?2026l\x1b[0m\x1b[?25h");
        for (sig, old) in &self.handlers {
            unsafe {
                libc::sigaction(*sig, old, std::ptr::null_mut());
            }
        }
        drop(self.guardian.take());
    }
}
struct Guardian {
    write: OwnedFd,
    pid: i32,
}
impl Guardian {
    fn start(original: libc::termios, flags: i32) -> io::Result<Self> {
        let mut pipe = [-1; 2];
        if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            unsafe {
                libc::close(pipe[0]);
                libc::close(pipe[1]);
            }
            return Err(io::Error::last_os_error());
        }
        if pid == 0 {
            unsafe {
                libc::close(pipe[1]);
                for sig in [
                    libc::SIGTTOU,
                    libc::SIGTTIN,
                    libc::SIGINT,
                    libc::SIGQUIT,
                    libc::SIGHUP,
                    libc::SIGTERM,
                ] {
                    libc::signal(sig, libc::SIG_IGN);
                }
                let mut shell = 0i32;
                let mut command = [0u8; 1];
                loop {
                    let n = libc::read(pipe[0], command.as_mut_ptr().cast(), 1);
                    if n < 0 {
                        if *libc::__errno_location() == libc::EINTR {
                            continue;
                        }
                        break;
                    }
                    if n == 0 {
                        break;
                    }
                    if command[0] == b'D' {
                        libc::_exit(0);
                    }
                    if command[0] == b'P' {
                        let mut raw = [0u8; 4];
                        let mut have = 0;
                        while have < 4 {
                            let n = libc::read(pipe[0], raw[have..].as_mut_ptr().cast(), 4 - have);
                            if n <= 0 {
                                break;
                            }
                            have += n as usize;
                        }
                        if have == 4 {
                            shell = i32::from_be_bytes(raw);
                        }
                    }
                }
                libc::tcsetattr(0, libc::TCSANOW, &original);
                libc::fcntl(0, libc::F_SETFL, flags);
                let reset=b"\x1b[?2026l\x1b[?1049l\x1b[0m\x1b[?25h\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l";
                libc::write(1, reset.as_ptr().cast(), reset.len());
                if shell > 1 {
                    libc::kill(-shell, libc::SIGHUP);
                    libc::kill(shell, libc::SIGHUP);
                }
                libc::_exit(0);
            }
        }
        unsafe {
            libc::close(pipe[0]);
        }
        Ok(Self {
            write: unsafe { OwnedFd::from_raw_fd(pipe[1]) },
            pid,
        })
    }
    fn child(&self, pid: u32) {
        let mut command = [b'P', 0, 0, 0, 0];
        command[1..].copy_from_slice(&(pid as i32).to_be_bytes());
        unsafe {
            libc::write(
                self.write.as_raw_fd(),
                command.as_ptr().cast(),
                command.len(),
            );
        }
    }
}
impl Drop for Guardian {
    fn drop(&mut self) {
        unsafe {
            libc::write(self.write.as_raw_fd(), b"D".as_ptr().cast(), 1);
        }
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            let n = unsafe { libc::waitpid(self.pid, std::ptr::null_mut(), libc::WNOHANG) };
            if n != 0 {
                break;
            }
            if Instant::now() > deadline {
                unsafe {
                    libc::kill(self.pid, libc::SIGKILL);
                    libc::waitpid(self.pid, std::ptr::null_mut(), 0);
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

pub struct Probe {
    pub origin: (u16, u16),
    pub sync: bool,
    pub initial_input: Vec<u8>,
    pub filter: ReplyFilter,
}
#[derive(Default)]
pub struct ReplyFilter {
    bytes: Vec<u8>,
    cpr: bool,
    mode: bool,
    pub origin: Option<(u16, u16)>,
    pub sync: bool,
}
impl ReplyFilter {
    fn new() -> Self {
        Self {
            cpr: true,
            mode: true,
            ..Self::default()
        }
    }
    pub fn done(&self) -> bool {
        !self.cpr && !self.mode
    }
    pub fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        if self.done() {
            return bytes.to_vec();
        }
        self.bytes.extend_from_slice(bytes);
        let mut out = vec![];
        while !self.bytes.is_empty() {
            if self.bytes[0] != 27 {
                out.push(self.bytes.remove(0));
                continue;
            }
            if self.bytes.len() == 1 {
                break;
            }
            if self.bytes[1] != b'[' {
                out.push(self.bytes.remove(0));
                continue;
            }
            if self.bytes.len() == 2 {
                break;
            }
            let end = self
                .bytes
                .iter()
                .enumerate()
                .skip(2)
                .find(|(_, b)| (0x40..=0x7e).contains(*b))
                .map(|(i, _)| i);
            let Some(end) = end else {
                if self.bytes.len() > 64 {
                    out.push(self.bytes.remove(0));
                    continue;
                }
                break;
            };
            let sequence = &self.bytes[2..=end];
            let s = std::str::from_utf8(sequence).unwrap_or("");
            let mut owned = false;
            if self.cpr
                && s.ends_with('R')
                && let Some((r, c)) = s[..s.len() - 1].split_once(';')
                && let (Ok(r), Ok(c)) = (r.parse::<u16>(), c.parse::<u16>())
                && r > 0
                && c > 0
            {
                self.origin = Some((r - 1, c - 1));
                self.cpr = false;
                owned = true;
            }
            if self.mode && s.starts_with("?2026;") && s.ends_with("$y") {
                let state = &s[6..s.len() - 2];
                self.sync = matches!(state, "1" | "2" | "3");
                self.mode = false;
                owned = true;
            }
            if !owned {
                out.extend_from_slice(&self.bytes[..=end]);
            }
            self.bytes.drain(..=end);
        }
        if self.done() {
            out.append(&mut self.bytes);
        }
        out
    }
    pub fn expire(&mut self) -> Vec<u8> {
        self.cpr = false;
        self.mode = false;
        std::mem::take(&mut self.bytes)
    }
}
pub fn probe(size: &libc::winsize) -> io::Result<Probe> {
    io::stdout().write_all(b"\x1b[6n\x1b[?2026$p")?;
    io::stdout().flush()?;
    let deadline = Instant::now() + Duration::from_millis(120);
    let mut filter = ReplyFilter::new();
    let mut input = vec![];
    while Instant::now() < deadline && !filter.done() {
        let mut p = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        let n = unsafe { libc::poll(&mut p, 1, 10) };
        if n > 0 {
            let mut bytes = [0; 4096];
            let n = unsafe { libc::read(0, bytes.as_mut_ptr().cast(), bytes.len()) };
            if n > 0 {
                input.extend(filter.push(&bytes[..n as usize]));
            }
        }
    }
    let origin = filter.origin.unwrap_or((size.ws_row - 1, 0));
    Ok(Probe {
        origin: (origin.0.min(size.ws_row - 1), origin.1.min(size.ws_col - 1)),
        sync: filter.sync,
        initial_input: input,
        filter,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replies_are_removed_but_keystrokes_and_paste_are_preserved() {
        let mut f = ReplyFilter::new();
        let mut out = vec![];
        for b in b"x\x1b[A\x1b[12;25R\x1b[?2026;2$y\x1b[200~paste\x1b[201~" {
            out.extend(f.push(&[*b]));
        }
        assert_eq!(f.origin, Some((11, 24)));
        assert!(f.sync);
        assert_eq!(out, b"x\x1b[A\x1b[200~paste\x1b[201~");
    }
}

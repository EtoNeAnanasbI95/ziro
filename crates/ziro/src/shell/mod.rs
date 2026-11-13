mod popup;
mod screen;
mod startup;
mod terminal;

use screen::Compositor;
use startup::Startup;
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{
            net::UnixStream,
            process::{CommandExt, ExitStatusExt},
        },
    },
    process::{Child, Command, Stdio},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use terminal::{Terminal, nonblock};
use ziro_proto::Framer;

struct ShellChild(Child);
impl Drop for ShellChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_some() {
            return;
        }
        let pid = self.0.id() as i32;
        unsafe {
            libc::kill(-pid, libc::SIGHUP);
            libc::kill(pid, libc::SIGHUP);
        }
        let deadline = Instant::now() + Duration::from_millis(150);
        loop {
            if self.0.try_wait().ok().flatten().is_some() {
                break;
            }
            if Instant::now() > deadline {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
                let _ = self.0.kill();
                let _ = self.0.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
pub fn run(no_rc: bool, login: bool) -> Result<i32, Box<dyn std::error::Error>> {
    if std::env::var("ZIRO_SHELL_ACTIVE").as_deref() == Ok("1") {
        return Err("ziro shell is already active; exit it before starting another wrapper".into());
    }
    let startup = Startup::create(no_rc)?;
    let mut size = terminal::size()?;
    let tty = Terminal::enter()?;
    let probe = terminal::probe(&size)?;
    let (mut renderer, child_renderer) = UnixStream::pair()?;
    renderer.set_nonblocking(true)?;
    child_renderer.set_nonblocking(true)?;
    let mut master = -1;
    let mut slave = -1;
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            &tty.original,
            &size,
        )
    } < 0
    {
        return Err(io::Error::last_os_error().into());
    }
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
    }
    nonblock(master.as_raw_fd())?;
    let render_fd = child_renderer.as_raw_fd();
    let executable = std::env::current_exe()?;
    let mut command = Command::new("zsh");
    command
        .arg(if login { "-il" } else { "-i" })
        .env("ZDOTDIR", &startup.directory)
        .env("ZIRO_RENDER_FD", "198")
        .env("ZIRO_SHELL_ACTIVE", "1")
        .env("ZIRO_BIN", &executable)
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave.try_clone()?));
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::dup2(render_fd, 198) < 0 || libc::fcntl(198, libc::F_SETFD, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = ShellChild(command.spawn()?);
    // Command keeps its Stdio copies of the slave alive; without dropping
    // them the master never reports EOF/EIO after the shell exits.
    drop(command);
    tty.child(child.0.id());
    drop(slave);
    drop(child_renderer);
    let mut display = Compositor::new(size.ws_row, size.ws_col, probe.origin, probe.sync);
    let mut replies = probe.filter;
    let replies_expire = Instant::now() + Duration::from_secs(1);
    let mut input: VecDeque<u8> = probe.initial_input.into();
    let mut framer = Framer::default();
    let mut render_open = true;
    let mut bytes = [0u8; 65536];
    let mut stdout = io::stdout().lock();
    let mut eof = false;
    let mut status = None;
    let mut exited_at: Option<Instant> = None;
    while !terminal::STOP.load(Ordering::Relaxed) {
        if terminal::RESIZED.swap(false, Ordering::Relaxed) {
            let next = terminal::size()?;
            stdout.write_all(&display.resize(next.ws_row, next.ws_col))?;
            size = next;
            unsafe {
                libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &size);
            }
        }
        if !replies.done() && Instant::now() > replies_expire {
            input.extend(replies.expire());
        }
        let mut fds = [
            libc::pollfd {
                fd: 0,
                events: if input.len() < 1024 * 1024 {
                    libc::POLLIN
                } else {
                    0
                },
                revents: 0,
            },
            libc::pollfd {
                fd: master.as_raw_fd(),
                events: libc::POLLIN | if input.is_empty() { 0 } else { libc::POLLOUT },
                revents: 0,
            },
            libc::pollfd {
                fd: if render_open {
                    renderer.as_raw_fd()
                } else {
                    -1
                },
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 20) } < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e.into());
        }
        if fds[0].revents & libc::POLLIN != 0 {
            loop {
                let n = unsafe { libc::read(0, bytes.as_mut_ptr().cast(), bytes.len()) };
                if n > 0 {
                    input.extend(replies.push(&bytes[..n as usize]));
                    if input.len() >= 1024 * 1024 {
                        break;
                    }
                } else if n == 0 {
                    eof = true;
                    break;
                } else {
                    let e = io::Error::last_os_error();
                    if e.kind() == io::ErrorKind::WouldBlock {
                        break;
                    }
                    if e.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(e.into());
                }
            }
        }
        if fds[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            eof = true;
        }
        if !input.is_empty() {
            let (first, _) = input.as_slices();
            let n = unsafe {
                libc::write(
                    master.as_raw_fd(),
                    first.as_ptr().cast(),
                    first.len().min(65536),
                )
            };
            if n > 0 {
                input.drain(..n as usize);
            } else if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::WouldBlock
                    && e.kind() != io::ErrorKind::Interrupted
                    && e.raw_os_error() != Some(libc::EIO)
                {
                    return Err(e.into());
                }
            }
        }
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            loop {
                let n = unsafe {
                    libc::read(master.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len())
                };
                if n > 0 {
                    display.push_child(&bytes[..n as usize]);
                    if (n as usize) < bytes.len() {
                        break;
                    }
                } else if n == 0 {
                    eof = true;
                    break;
                } else {
                    let e = io::Error::last_os_error();
                    if e.kind() == io::ErrorKind::WouldBlock {
                        break;
                    }
                    if e.raw_os_error() == Some(libc::EIO) {
                        eof = true;
                        break;
                    }
                    if e.kind() != io::ErrorKind::Interrupted {
                        return Err(e.into());
                    }
                }
            }
        }
        if render_open && fds[2].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            loop {
                match renderer.read(&mut bytes) {
                    Ok(0) => {
                        render_open = false;
                        display.ui.editing = false;
                        display.ui.popup = None;
                        break;
                    }
                    Ok(n) => match framer.push(&bytes[..n]) {
                        Ok(messages) => {
                            for m in messages {
                                display.apply(&m);
                            }
                        }
                        Err(_) => {
                            render_open = false;
                            display.ui.popup = None;
                            break;
                        }
                    },
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        render_open = false;
                        display.ui.popup = None;
                        break;
                    }
                }
            }
        }
        stdout.write_all(&display.flush())?;
        stdout.flush()?;
        if status.is_none()
            && let Some(exit) = child.0.try_wait()?
        {
            status = Some(exit);
            exited_at = Some(Instant::now());
        }
        // A background job may still hold the PTY open after the shell
        // exits. Drain its last output briefly, then leave like a terminal.
        if eof || exited_at.is_some_and(|t| t.elapsed() > Duration::from_millis(100)) {
            break;
        }
    }
    stdout.write_all(&display.close())?;
    if display.parser.screen().alternate_screen() {
        stdout.write_all(b"\x1b[?1049l")?;
    }
    stdout.flush()?;
    drop(stdout);
    if status.is_none() {
        status = child.0.try_wait()?;
    }
    let code = status
        .map(|s| s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(1)))
        .unwrap_or(143);
    drop(child);
    drop(master);
    drop(tty);
    drop(startup);
    Ok(code)
}

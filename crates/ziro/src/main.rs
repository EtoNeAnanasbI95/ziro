mod daemon;
mod shell;
use clap::{Parser, Subcommand};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
};
#[derive(Parser)]
#[command(version, about = "Live zsh completion menu and history suggestions")]
struct Cli {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    #[command(hide = true)]
    SocketNonblock,
    Daemon {
        #[arg(long)]
        detach: bool,
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    Init {
        #[arg(value_parser=["zsh"])]
        shell: String,
    },
    Shell {
        #[arg(long)]
        no_rc: bool,
        #[arg(long)]
        login: bool,
    },
    Doctor,
    Reload,
    Stats {
        #[arg(long)]
        json: bool,
    },
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let original_umask = unsafe { libc::umask(0o077) };
    let cli = Cli::parse();
    match cli.command {
        Action::Shell { no_rc, login } => {
            unsafe {
                libc::umask(original_umask);
            }
            let code = shell::run(no_rc, login)?;
            std::process::exit(code);
        }
        Action::SocketNonblock => {
            // File status flags belong to the shared open file description:
            // stdin is a dup of zsh's socket, so this also makes its fd nonblocking.
            let flags = unsafe { libc::fcntl(0, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(0, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }

        Action::Reload => {
            let pid: i32 = std::fs::read_to_string(
                daemon::socket_path().parent().unwrap().join("daemon.lock"),
            )?
            .trim()
            .parse()?;
            if pid <= 1 || unsafe { libc::kill(pid, libc::SIGHUP) } != 0 {
                return Err("cannot signal daemon".into());
            }
        }
        Action::Daemon { detach, socket } => {
            if detach {
                let ready_socket = socket.clone().unwrap_or_else(daemon::socket_path);
                let mut c = Command::new(std::env::current_exe()?);
                c.arg("daemon");
                if let Some(path) = socket {
                    c.arg("--socket").arg(path);
                }
                std::fs::create_dir_all(daemon::state_dir())?;
                let log = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(daemon::state_dir().join("daemon.log"))?;
                c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
                use std::os::unix::process::CommandExt;
                unsafe {
                    c.pre_exec(|| {
                        if libc::setsid() < 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                let mut child = c.spawn()?;
                for _ in 0..100 {
                    if std::os::unix::net::UnixStream::connect(&ready_socket).is_ok() {
                        return Ok(());
                    }
                    if let Some(status) = child.try_wait()? {
                        return Err(format!("daemon exited: {status}").into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                return Err("daemon did not become ready".into());
            }
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_env("ZIRO_LOG")
                        .unwrap_or_else(|_| "warn".into()),
                )
                .with_writer(std::io::stderr)
                .init();
            let (config, warnings) = ziro_core::config::Config::load(&daemon::config_path())?;
            for warning in warnings {
                tracing::warn!(%warning);
            }
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            tokio::task::LocalSet::new().block_on(
                &rt,
                daemon::serve(&socket.unwrap_or_else(daemon::socket_path), config),
            )?;
        }
        Action::Init { .. } => {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../plugin/ziro.plugin.zsh")
                .canonicalize()?;
            println!(
                "source '{}'",
                path.display().to_string().replace('\'', "'\\''")
            );
        }
        Action::Doctor => {
            let output=Command::new("zsh").args(["-fc","print -r -- $ZSH_VERSION; zmodload zsh/zpty && zmodload zsh/net/socket && zmodload zsh/system && zmodload zsh/datetime && print modules-ok"]).output()?;
            print!("{}", String::from_utf8_lossy(&output.stdout));
            println!(
                "socket: {} ({})",
                daemon::socket_path().display(),
                if std::os::unix::net::UnixStream::connect(daemon::socket_path()).is_ok() {
                    "reachable"
                } else {
                    "offline"
                }
            );
            let rc = std::env::var_os("ZDOTDIR")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".zshrc");
            if let Ok(text) = std::fs::read_to_string(rc) {
                for name in ["zsh-autosuggestions", "zsh-autocomplete"] {
                    if text.contains(name) {
                        println!("check conflicting plugin: {name}");
                    }
                }
                if let (Some(ziro), Some(omz)) =
                    (text.find("ziro.plugin.zsh"), text.find("oh-my-zsh.sh"))
                    && ziro < omz
                {
                    println!("load ziro after oh-my-zsh");
                }
            }
        }
        Action::Stats { json } => {
            let path = daemon::state_dir().join("stats.json");
            let text = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".into());
            if json {
                println!("{text}");
            } else {
                let v: serde_json::Value = serde_json::from_str(&text)?;
                println!("{}", serde_json::to_string_pretty(&v)?);
            }
        }
    }
    Ok(())
}

use std::{
    fs, io,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
pub struct Startup {
    pub directory: PathBuf,
    pub user_directory: PathBuf,
    had_zdotdir: bool,
}
/// $ZIRO_PLUGIN, then a plugin next to an installed binary
/// (`<prefix>/bin/ziro` → `<prefix>/share/ziro/plugin`), then the source tree.
fn plugin_path() -> io::Result<PathBuf> {
    let mut candidates = vec![];
    if let Some(p) = std::env::var_os("ZIRO_PLUGIN") {
        candidates.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
    {
        candidates.push(bin.join("../share/ziro/plugin/ziro.plugin.zsh"));
        // target/{debug,release}/ziro inside the repository.
        candidates.push(bin.join("../../plugin/ziro.plugin.zsh"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugin/ziro.plugin.zsh"));
    candidates
        .into_iter()
        .find_map(|p| p.canonicalize().ok().filter(|p| p.is_file()))
        .ok_or_else(|| io::Error::other("ziro.plugin.zsh not found; set ZIRO_PLUGIN"))
}
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
impl Startup {
    pub fn create(no_rc: bool) -> io::Result<Self> {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("ziro-shell-{}-{id}", std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let base = std::env::var_os("ZDOTDIR")
            .or_else(|| std::env::var_os("HOME"))
            .ok_or_else(|| io::Error::other("HOME/ZDOTDIR is not set"))?;
        let user_directory = if Path::new(&base).is_absolute() {
            PathBuf::from(base)
        } else {
            std::env::current_dir()?.join(base)
        };
        let plugin = plugin_path()?;
        let mut startup = Self {
            directory,
            user_directory,
            had_zdotdir: std::env::var_os("ZDOTDIR").is_some(),
        };
        if let Err(e) = startup.write(no_rc, &plugin) {
            let _ = fs::remove_dir_all(&startup.directory);
            return Err(e);
        }
        Ok(startup)
    }
    fn write(&mut self, no_rc: bool, plugin: &Path) -> io::Result<()> {
        let original = quote(&self.user_directory);
        let bootstrap = quote(&self.directory);
        fs::write(
            self.directory.join(".zshenv"),
            format!(
                "typeset -gx ZIRO_USER_ZDOTDIR={original}\nZDOTDIR=$ZIRO_USER_ZDOTDIR\n{}\ntypeset -gx ZIRO_USER_ZDOTDIR=${{ZDOTDIR:-$HOME}}\nZDOTDIR={bootstrap}\n",
                if no_rc {
                    ""
                } else {
                    "[[ -r $ZDOTDIR/.zshenv ]] && builtin source \"$ZDOTDIR/.zshenv\""
                }
            ),
        )?;
        fs::write(
            self.directory.join(".zprofile"),
            format!(
                "ZDOTDIR=$ZIRO_USER_ZDOTDIR\n{}\nZIRO_USER_ZDOTDIR=${{ZDOTDIR:-$HOME}}\nZDOTDIR={bootstrap}\n",
                if no_rc {
                    ""
                } else {
                    "[[ -r $ZDOTDIR/.zprofile ]] && builtin source \"$ZDOTDIR/.zprofile\""
                }
            ),
        )?;
        fs::write(
            self.directory.join(".zshrc"),
            format!(
                "ZDOTDIR=$ZIRO_USER_ZDOTDIR\n{}\n(( $+functions[_ziro_precmd] )) || builtin source {}\n{}unset ZIRO_USER_ZDOTDIR\n",
                if no_rc {
                    ""
                } else {
                    "[[ -r $ZDOTDIR/.zshrc ]] && builtin source \"$ZDOTDIR/.zshrc\""
                },
                quote(plugin),
                // Do not leak ZDOTDIR=$HOME to children when it was unset.
                if self.had_zdotdir {
                    ""
                } else {
                    "[[ $ZDOTDIR == \"$HOME\" ]] && unset ZDOTDIR\n"
                }
            ),
        )?;
        Ok(())
    }
}
impl Drop for Startup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

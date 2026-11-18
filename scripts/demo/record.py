#!/usr/bin/env python3
"""Record docs/demo.gif: drive an isolated zsh+ziro in tmux, snapshot the
screen after every step, render snapshots with headless Chrome, glue with ffmpeg.

Nothing from the user's shell leaks into the GIF: history, ziro state, config
and the working directory are all temporary; the git repo is a fake one.

Needs: tmux, zsh, oh-my-zsh, installed ziro (`ziro` in PATH and its plugin),
google-chrome-stable (or chromium), ffmpeg, JetBrains Mono, Noto Color Emoji.

    python3 scripts/demo/record.py [out.gif]
"""
import html
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "docs/demo.gif"
COLS, ROWS = 88, 13
SOCK = "zirodemo"

PALETTE = [  # Tokyo Night
    "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6",
    "#565f89", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
]
FG, BG = "#c0caf5", "#1a1b26"
BG_OVERRIDE = {PALETTE[4]: "#3d59a1"}  # selection: darker blue keeps text readable


def sh(*args, **kw):
    return subprocess.run(args, check=True, text=True, capture_output=True, **kw).stdout


def tmux(*args):
    return sh("tmux", "-L", SOCK, *args)


# ---------------------------------------------------------------- environment
def setup(tmp: Path) -> dict:
    demo = tmp / "ziro-demo"
    (demo / "src").mkdir(parents=True)
    (demo / "infra").mkdir()
    (demo / "docs").mkdir()
    (demo / "README.md").write_text("# demo\n")
    (demo / "src/main.rs").write_text("fn main() {}\n")
    (demo / "infra/dev.yml").write_text("services: {}\n")
    git = ["git", "-C", str(demo), "-c", "user.name=demo", "-c", "user.email=demo@example.com"]
    sh(*git, "init", "-q", "-b", "main")
    sh(*git, "add", "-A")
    sh(*git, "commit", "-qm", "init")
    for b in ["feat/menu-under-cursor", "fix/tab-cycling", "release/0.2"]:
        sh(*git, "branch", b)

    zdot = tmp / "zdot"
    zdot.mkdir()
    now = int(time.time())
    hist = [
        "docker compose -f infra/dev.yml up -d",
        "docker compose -f infra/dev.yml logs -f api",
        "docker compose -f infra/dev.yml up -d",
        "cargo build --release",
        "git status",
    ]
    (zdot / ".zsh_history").write_text(
        "".join(f": {now - 3600 + i * 60}:0;{c}\n" for i, c in enumerate(hist))
    )
    plugin = sh("ziro", "init", "zsh").strip()
    (zdot / ".zshrc").write_text(
        f"""export ZSH=$HOME/.oh-my-zsh
ZSH_THEME=robbyrussell
DISABLE_AUTO_UPDATE=true
ZSH_COMPDUMP=$ZDOTDIR/.zcompdump
plugins=(git zsh-syntax-highlighting)
source $ZSH/oh-my-zsh.sh
HISTFILE=$ZDOTDIR/.zsh_history
SAVEHIST=1000 HISTSIZE=1000
{plugin}
"""
    )
    cfg = tmp / "config/ziro"
    cfg.mkdir(parents=True)
    (cfg / "config.toml").write_text(
        """[menu]
enter_accepts = "auto"
[keys]
accept = ""
next = ["^I", "^[[B", "^[OB"]
prev = ["^[[Z", "^[[A", "^[OA"]
"""
    )
    env = {
        "ZDOTDIR": str(zdot),
        "HISTFILE": str(zdot / ".zsh_history"),
        "XDG_STATE_HOME": str(tmp / "state"),
        "XDG_CONFIG_HOME": str(tmp / "config"),
        "XDG_CACHE_HOME": str(tmp / "cache"),
        "ZIRO_SOCKET": str(tmp / "daemon.sock"),
        "TERM": "xterm-256color",
    }
    return {"cwd": demo, "env": env}


# ------------------------------------------------------------------ recording
class Recorder:
    def __init__(self):
        self.frames = []  # [screen, cursor, seconds]

    def snap(self, hold):
        screen = tmux("capture-pane", "-p", "-e", "-t", "demo")
        cur = tuple(map(int, tmux("display", "-p", "-t", "demo", "#{cursor_x} #{cursor_y}").split()))
        if self.frames and self.frames[-1][0] == screen and self.frames[-1][1] == cur:
            self.frames[-1][2] += hold
        else:
            self.frames.append([screen, cur, hold])

    def watch(self, seconds, step=0.08):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            time.sleep(step)
            self.snap(step)

    def keys(self, *keys, hold=0.6):
        tmux("send-keys", "-t", "demo", *keys)
        self.watch(hold)

    def type(self, text, delay=0.09):
        for ch in text:
            tmux("send-keys", "-t", "demo", "-l", ch)
            self.watch(delay, step=delay)


def scenario(r: Recorder):
    r.watch(0.8)
    # Subcommands with descriptions, first item preselected.
    r.type("git ch")
    r.watch(1.4)
    r.keys("Tab")
    r.keys("Tab")
    r.keys("BTab")
    r.keys("BTab", hold=0.8)
    # Enter inserts the item, the next menu (branches) appears at once.
    r.keys("Enter", hold=1.2)
    r.keys("Tab")
    r.keys("Tab", hold=0.8)
    r.keys("Enter", hold=1.4)
    r.keys("C-u", hold=0.5)
    # Options with descriptions, fuzzy filtering.
    r.type("tar -")
    r.watch(1.2)
    r.type("x")
    r.watch(1.0)
    r.keys("C-u", hold=0.5)
    # Ghost text from history, accepted with →.
    r.type("docker compose -f")
    r.watch(1.2)
    r.keys("Right", hold=1.6)
    r.keys("C-u", hold=0.5)
    # Big completion lists still come from zsh itself.
    r.type("pacman -S fire")
    r.watch(4.0)
    r.keys("C-u", hold=1.0)


# ------------------------------------------------------------------ rendering
SGR = re.compile(r"\x1b\[([0-9;]*)m")


def color256(n):
    if n < 16:
        return PALETTE[n]
    if n < 232:
        n -= 16
        v = [0, 95, 135, 175, 215, 255]
        return "#%02x%02x%02x" % (v[n // 36], v[n // 6 % 6], v[n % 6])
    g = 8 + (n - 232) * 10
    return "#%02x%02x%02x" % (g, g, g)


def apply(st, params):
    p = [int(x) if x else 0 for x in params.split(";")] if params else [0]
    i = 0
    while i < len(p):
        c = p[i]
        if c == 0:
            st.update(fg=None, bg=None, bold=False, rev=False, ul=False)
        elif c == 1:
            st["bold"] = True
        elif c == 22:
            st["bold"] = False
        elif c == 4:
            st["ul"] = True
        elif c == 24:
            st["ul"] = False
        elif c == 7:
            st["rev"] = True
        elif c == 27:
            st["rev"] = False
        elif 30 <= c <= 37:
            st["fg"] = PALETTE[c - 30]
        elif 90 <= c <= 97:
            st["fg"] = PALETTE[c - 90 + 8]
        elif 40 <= c <= 47:
            st["bg"] = PALETTE[c - 40]
        elif 100 <= c <= 107:
            st["bg"] = PALETTE[c - 100 + 8]
        elif c == 39:
            st["fg"] = None
        elif c == 49:
            st["bg"] = None
        elif c in (38, 48) and i + 1 < len(p):
            key = "fg" if c == 38 else "bg"
            if p[i + 1] == 5 and i + 2 < len(p):
                st[key] = color256(p[i + 2])
                i += 2
            elif p[i + 1] == 2 and i + 4 < len(p):
                st[key] = "#%02x%02x%02x" % tuple(p[i + 2 : i + 5])
                i += 4
        i += 1


def width(ch):
    if unicodedata.combining(ch):
        return 0
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


def cells(line):
    """[(char, width, style)] for one captured line."""
    st = dict(fg=None, bg=None, bold=False, rev=False, ul=False)
    out, pos = [], 0
    for m in SGR.finditer(line + "\x1b[m"):
        for ch in line[pos : m.start()]:
            w = width(ch)
            if w:
                out.append((ch, w, dict(st)))
        apply(st, m.group(1))
        pos = m.end()
    return out


def render_html(screen, cursor):
    rows = screen.split("\n")[:ROWS]
    rows += [""] * (ROWS - len(rows))
    body = []
    for y, line in enumerate(rows):
        cs = cells(line)
        col, parts = 0, []
        for ch, w, st in cs:
            fg, bg = st["fg"] or FG, st["bg"]
            if st["rev"]:
                fg, bg = bg or BG, fg
            bg = BG_OVERRIDE.get(bg, bg)
            if (col, y) == cursor:
                fg, bg = BG, "#c0caf5"
            style = f"color:{fg};" + (f"background:{bg};" if bg else "")
            if st["bold"]:
                style += "font-weight:700;"
            if st["ul"]:
                style += "text-decoration:underline;"
            style += f"width:{w}ch;"
            parts.append(f'<span style="{style}">{html.escape(ch)}</span>')
            col += w
        if cursor[1] == y and cursor[0] >= col:
            parts.append("<span>" + " " * (cursor[0] - col) + "</span>")
            parts.append('<span style="background:#c0caf5;width:1ch"> </span>')
        body.append("<div>" + "".join(parts) + "</div>")
    return f"""<!doctype html><meta charset="utf-8"><style>
html,body{{margin:0;background:#0d0e14}}
.win{{margin:18px;border-radius:12px;background:{BG};box-shadow:0 10px 30px #000a;overflow:hidden;display:inline-block}}
.bar{{height:30px;display:flex;align-items:center;gap:8px;padding-left:14px;background:#16161e}}
.bar i{{width:12px;height:12px;border-radius:50%;display:block}}
.term{{width:{COLS}ch;padding:14px 18px 18px;font:15px/1.32 'JetBrainsMono Nerd Font Mono','JetBrains Mono','Noto Color Emoji',monospace;color:{FG};white-space:pre}}
.term div{{height:1.32em}}
.term span{{display:inline-block;overflow:hidden;text-align:center;vertical-align:top;height:1.32em}}
</style><div class="win"><div class="bar"><i style="background:#f7768e"></i><i style="background:#e0af68"></i><i style="background:#9ece6a"></i></div>
<div class="term">{"".join(body)}</div></div>"""


def chrome():
    for c in ("google-chrome-stable", "chromium", "google-chrome"):
        if shutil.which(c):
            return c
    sys.exit("need google-chrome-stable or chromium")


def main():
    tmp = Path(tempfile.mkdtemp(prefix="ziro-demo-"))
    try:
        cfg = setup(tmp)
        env = " ".join(f"{k}={v}" for k, v in cfg["env"].items())
        subprocess.run(["tmux", "-L", SOCK, "kill-server"], capture_output=True)
        tmux("-f", "/dev/null", "new-session", "-d", "-s", "demo", "-x", str(COLS), "-y", str(ROWS),
             "-c", str(cfg["cwd"]), f"env -u ISTERM {env} zsh -i")
        time.sleep(3)
        tmux("send-keys", "-t", "demo", "clear", "Enter")
        time.sleep(0.8)
        r = Recorder()
        scenario(r)
        frames_dir = tmp / "frames"
        frames_dir.mkdir()
        browser, profile = chrome(), tmp / "chrome"
        concat = []
        for i, (screen, cur, hold) in enumerate(r.frames):
            page = frames_dir / f"{i:04}.html"
            page.write_text(render_html(screen, cur))
            png = frames_dir / f"{i:04}.png"
            subprocess.run([browser, "--headless=new", "--disable-gpu", "--hide-scrollbars",
                            f"--user-data-dir={profile}", "--force-device-scale-factor=1",
                            f"--screenshot={png}", "--window-size=1000,560", page.as_uri()],
                           check=True, capture_output=True)
            concat += [f"file '{png}'", f"duration {max(hold, 0.04):.2f}"]
        concat.append(f"file '{frames_dir / f'{len(r.frames) - 1:04}.png'}'")
        (tmp / "list.txt").write_text("\n".join(concat) + "\n")
        OUT.parent.mkdir(parents=True, exist_ok=True)
        # Crop to the window (+ a margin for its shadow).
        w, h, x, y = map(int, re.match(r"(\d+)x(\d+)\+(\d+)\+(\d+)", sh(
            "magick", str(frames_dir / "0000.png"), "-fuzz", "2%", "-format", "%@", "info:")).groups())
        m = 14
        crop = f"crop={w + 2 * m}:{h + 2 * m}:{max(x - m, 0)}:{max(y - m, 0)}"
        sh("ffmpeg", "-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", str(tmp / "list.txt"),
           "-vf", f"{crop},split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=none",
           "-fps_mode", "vfr", str(OUT))
        print(f"{OUT}: {len(r.frames)} frames, {OUT.stat().st_size // 1024} KiB")
    finally:
        subprocess.run(["tmux", "-L", SOCK, "kill-server"], capture_output=True)
        sock = str(tmp / "daemon.sock")
        pids = subprocess.run(["pgrep", "-f", f"daemon --socket {sock}"], capture_output=True, text=True).stdout.split()
        for pid in pids:
            if int(pid) != os.getpid():
                subprocess.run(["kill", pid])
        if os.environ.get("ZIRO_DEMO_KEEP"):
            print("kept", tmp)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""End-to-end check of `ziro shell` inside a real terminal emulator (tmux).

Isolated HOME, socket and state; never touches the user's daemon or rc files.
"""
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.environ.get("ZIRO_BIN") or os.path.join(ROOT, "target/debug/ziro")
SOCK = "ziro-overlay-smoke"


def tmux(*args, check=True):
    return subprocess.run(["tmux", "-L", SOCK, "-f", "/dev/null", *args],
                          check=check, capture_output=True, text=True).stdout


def screen():
    return tmux("capture-pane", "-p", "-t", "t")


def keys(*k, wait=0.0):
    tmux("send-keys", "-t", "t", *k)
    if wait:
        time.sleep(wait)


def until(pred, what, timeout=5.0):
    end = time.time() + timeout
    while time.time() < end:
        s = screen()
        if pred(s):
            return s
        time.sleep(0.05)
    print(screen())
    raise SystemExit(f"FAIL: {what}")


def start(home, run, cols=100, rows=30):
    tmux("kill-server", check=False)
    env = (f"env -i HOME={home} TERM=tmux-256color PATH={os.environ['PATH']} "
           f"XDG_RUNTIME_DIR={run} XDG_STATE_HOME={home}/.state ZIRO_SOCKET={run}/d.sock")
    tmux("new-session", "-d", "-x", str(cols), "-y", str(rows), "-s", "t", "-c", home,
         f"{env} {BIN} shell; echo EXIT=$?; sleep 30")
    until(lambda s: "~ %" in s, "prompt")


def pane_tty():
    return tmux("display", "-p", "-t", "t", "#{pane_tty}").strip()


def wrapper_pid():
    pane = tmux("display", "-p", "-t", "t", "#{pane_pid}").strip()
    out = subprocess.run(["pgrep", "-P", pane, "-f", f"{BIN} shell"],
                         capture_output=True, text=True).stdout.split()
    return int(out[0])


def raw_mode():
    return "-icanon" in subprocess.run(["stty", "-F", pane_tty(), "-a"],
                                       capture_output=True, text=True).stdout


def main():
    if not shutil.which("tmux"):
        raise SystemExit("SKIP: tmux not installed")
    tmp = tempfile.mkdtemp(prefix="ziro-overlay-")
    home, run = f"{tmp}/home", f"{tmp}/run"
    os.makedirs(f"{home}/Documents/sub")
    os.makedirs(f"{home}/Downloads")
    os.makedirs(f"{home}/My Dir")
    os.makedirs(run, mode=0o700)
    with open(f"{home}/.zshrc", "w") as f:
        f.write("PS1='%~ %# '\nautoload -Uz compinit && compinit -u\n"
                "alias zzal='echo alias'\nzzfn() { :; }\n")
    try:
        start(home, run)
        # Popup follows the cursor, below it, with directory icons.
        keys("cd D")
        s = until(lambda s: "Documents/" in s and "Downloads/" in s and "╭" in s, "cd popup")
        lines = s.splitlines()
        top = next(i for i, l in enumerate(lines) if "╭" in l)
        assert top == 1 and lines[top].index("╭") >= 4, s
        assert "📁" in s, s
        # Tab inserts, popup shows the directory contents, Enter runs.
        keys("Tab")
        until(lambda s: "cd Documents/" in s and "sub/" in s, "Tab inserts dir and lists contents")
        keys("Enter")
        until(lambda s: "~/Documents %" in s and "╭" not in s, "Enter runs, popup removed")
        # Kinds from the live shell.
        keys("C-u", "zz")
        until(lambda s: "🔗 zzal  alias" in s and "🧩 zzfn  function" in s, "alias/function kinds")
        # No duplicate rows for a builtin that is also a binary.
        keys("C-u", "echo")
        s = until(lambda s: "builtin" in s, "echo popup")
        assert sum("echo  builtin" in l for l in s.splitlines()) == 1, s
        keys("Escape", "C-u")
        until(lambda s: "╭" not in s, "Escape hides popup")
        # Bottom of the screen: popup opens above, without destroying output.
        keys("cd ~; seq 60", "Enter")
        until(lambda s: "60" in s, "seq output")
        keys("cd D")
        s = until(lambda s: "Downloads/" in s, "popup above")
        lines = [l for l in s.splitlines()]
        prompt_row = max(i for i, l in enumerate(lines) if l.startswith("~ % cd D"))
        assert any("╰" in l for l in lines[:prompt_row]), s
        keys("C-u")
        until(lambda s: "╰" not in s, "popup removed")
        assert "57" in screen(), screen()
        # Resize keeps the popup and re-places it.
        keys("cd D")
        until(lambda s: "Downloads/" in s, "popup before resize")
        tmux("resize-window", "-t", "t", "-x", "50", "-y", "20")
        until(lambda s: "Downloads/" in s and all(len(l) <= 50 for l in s.splitlines()), "popup after resize")
        keys("C-u")
        # Alternate screen: no popup inside full-screen programs.
        keys("vim -u NONE -N", "Enter", wait=0.5)
        keys("icd D", wait=0.8)
        assert "Downloads/" not in screen(), screen()
        keys("Escape", ":q!", "Enter")
        until(lambda s: s.rstrip().endswith("~ %"), "back from vim")
        # Ctrl+C and job control.
        keys("sleep 30", "Enter", wait=0.3)
        keys("C-c", wait=0.2)
        keys("echo rc=$?", "Enter")
        until(lambda s: "rc=130" in s, "Ctrl+C reaches the job")
        keys("sleep 30", "Enter", wait=0.3)
        keys("C-z")
        until(lambda s: "suspended" in s, "Ctrl+Z")
        keys("kill %1", "Enter")
        # Exit code and terminal restoration.
        keys("exit 7", "Enter")
        until(lambda s: "EXIT=7" in s, "exit status")
        assert not raw_mode(), "termios not restored"
        # SIGKILL of the wrapper: guardian restores tty and ends the shell.
        for sig, code in ((signal.SIGTERM, 143), (signal.SIGKILL, 137)):
            start(home, run)
            pid = wrapper_pid()
            assert raw_mode()
            os.kill(pid, sig)
            until(lambda s: f"EXIT={code}" in s, f"exit after {sig.name}")
            time.sleep(0.2)
            assert not raw_mode(), f"termios not restored after {sig.name}"
        print("PASS: overlay popup placement/above/resize, Tab/Enter/Escape, kinds, dedup, "
              "alt screen, Ctrl+C/Ctrl+Z, exit code, SIGTERM/SIGKILL restore")
    finally:
        tmux("kill-server", check=False)
        subprocess.run(["pkill", "-f", f"{run}/d.sock"], check=False)
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()

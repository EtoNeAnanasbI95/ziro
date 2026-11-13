#!/usr/bin/env python3
"""Real ZLE/PTTY smoke, with temporary config/state and no user rc edits."""
import fcntl
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time
ROOT = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='ziro-shell-') as tmp:
    env = os.environ | {'ZDOTDIR': tmp, 'XDG_STATE_HOME': tmp + '/state', 'XDG_CONFIG_HOME': tmp + '/config', 'ZIRO_SOCKET': tmp + '/daemon.sock', 'TERM': 'xterm-256color', 'ZIRO_BIN': str(ROOT / 'target/debug/ziro'), 'ZIRO_TEST_STATE': tmp + '/editor'}
    env.pop('ISTERM', None)
    setup = "autoload -Uz compinit\ncompinit -D"
    if os.environ.get('ZIRO_TEST_OMZ') == '1':
        setup = "ZSH=$HOME/.oh-my-zsh\nZSH_THEME=''\nZSH_COMPDUMP=$ZDOTDIR/.zcompdump\nDISABLE_AUTO_UPDATE=true\nplugins=(git zsh-syntax-highlighting fast-syntax-highlighting zsh-autosuggestions)\nsource $ZSH/oh-my-zsh.sh"
    Path(tmp+'/My Docs').mkdir()
    Path(tmp+"/файл'1.txt").write_text('fixture')
    Path(tmp+'/one.md').write_text('fixture')
    Path(tmp + '/.zshrc').write_text(f'''
{setup}
bindkey -e
_ziro_test_custom() {{ local -a descriptions=('alpha -- custom alpha' 'beta -- custom beta'); compadd -d descriptions -- alpha beta; }}
compdef _ziro_test_custom ziro-custom
_ziro_test_slow() {{ sleep 2; compadd late; }}
compdef _ziro_test_slow slowcmd
PROMPT='ZIRO> '
RPROMPT=''
source '{ROOT}/plugin/ziro.plugin.zsh'
_ziro_test_snapshot() {{ print -rl -- "$BUFFER" "$CURSOR" "$_ziro_visible" "$_ziro_ghost" > $ZIRO_TEST_STATE; }}
zle -N _ziro_test_snapshot
bindkey '^X^X' _ziro_test_snapshot
''')
    daemon = subprocess.Popen([str(ROOT / 'target/debug/ziro'), 'daemon', '--socket', env['ZIRO_SOCKET']], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 100, 0, 0))
    def attach_tty():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
    tmux_socket = 'ziro-smoke-'+str(os.getpid())
    shell_command=['zsh','-i']
    if os.environ.get('ZIRO_TEST_TMUX')=='1':
        shell_command=['tmux','-L',tmux_socket,'-f','/dev/null','new-session','-s','ziro','zsh -i']
    shell = subprocess.Popen(shell_command, stdin=slave, stdout=slave, stderr=slave, env=env, preexec_fn=attach_tty)
    os.close(slave)
    output = bytearray()
    def drain(seconds):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], min(.03, max(0, end-time.monotonic())))[0]:
                try: output.extend(os.read(master, 65536))
                except OSError: break
    def send(text, wait=.5):
        os.write(master, text.encode()); drain(wait)
    def snapshot():
        send('\x18\x18', .2)
        return Path(env['ZIRO_TEST_STATE']).read_text().splitlines()
    try:
        drain(1)
        send('git ch', 1)
        state = snapshot()
        assert state[0] == 'git ch', state
        assert state[2] == '1', (state, output.decode(errors='replace')[-4000:])
        send('\t', .5)
        state = snapshot()
        assert state[0].startswith('git checkout '), state
        send('\x03', .3)
        send('echo hello\r', .5)
        send('echo h', .4)
        state = snapshot()
        assert state[0] == 'echo h', state
        assert state[3] == 'ello', state
        send('\x1b[C', .3)
        assert snapshot()[0] == 'echo hello'
        send('\x03', .2)
        send('ziro-custom ', .5)
        assert snapshot()[2]=='1'
        send('\x1b[B', .2)
        send('\r', .2)
        assert snapshot()[0]=='ziro-custom beta ', snapshot()
        send('\x03', .2)
        send('cat '+tmp+'/My', .5)
        assert snapshot()[2]=='1', snapshot()
        send('\t', .2)
        assert snapshot()[0]=='cat '+tmp+'/My\\ Docs/', snapshot()
        send('\x03', .2)
        send('cat '+tmp+'/ф', .5)
        send('\t', .2)
        assert snapshot()[0]=="cat "+tmp+"/файл\\'1.txt ", snapshot()
        send('\x03', .2)
        send('echo '+tmp+'/*.md', .3)
        assert snapshot()[2]=='0'
        send('\t', .2)
        assert '*.md' not in snapshot()[0], snapshot()
        send('\x03', .2)
        send('slowcmd ', .05)
        send('typing', .05)
        assert snapshot()[0]=='slowcmd typing', snapshot()
        drain(.6)
        assert snapshot()[2]=='0'
        send('\x03', .3)
        send('ziro-custom ', .5)
        assert snapshot()[2]=='1', snapshot()
        send('\x03', .2)
        print('PASS: menu, Tab, Enter, history, ghost, Ctrl+C, custom compdef, spaces/quotes/Unicode, glob fallback, slow worker isolation/recovery')
    except BaseException:
        print(output.decode(errors='replace')[-1000:])
        raise
    finally:
        shell.terminate()
        try: shell.wait(timeout=2)
        except subprocess.TimeoutExpired: shell.kill(); shell.wait()
        if os.environ.get('ZIRO_TEST_TMUX')=='1':
            subprocess.run(['tmux','-L',tmux_socket,'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        daemon.terminate(); daemon.wait(timeout=2)
        errors=daemon.stderr.read().decode()
        if errors: print(errors)
        os.close(master)

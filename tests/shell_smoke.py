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
    key_setup='bindkey -v' if os.environ.get('ZIRO_TEST_VI')=='1' else 'bindkey -e'
    setup = "autoload -Uz compinit\ncompinit -D"
    if os.environ.get('ZIRO_TEST_OMZ') == '1':
        setup = "ZSH=$HOME/.oh-my-zsh\nZSH_THEME=''\nZSH_COMPDUMP=$ZDOTDIR/.zcompdump\nDISABLE_AUTO_UPDATE=true\nplugins=(git zsh-syntax-highlighting fast-syntax-highlighting zsh-autosuggestions)\nsource $ZSH/oh-my-zsh.sh"
    if os.environ.get('ZIRO_TEST_EXPLICIT')=='1':
        Path(tmp+'/config/ziro').mkdir(parents=True)
        Path(tmp+'/config/ziro/config.toml').write_text('[keys]\narrows="explicit"\n')
    Path(tmp+'/My Docs').mkdir()
    Path(tmp+"/файл'1.txt").write_text('fixture')
    Path(tmp+'/one.md').write_text('fixture')
    Path(tmp + '/.zshrc').write_text(f'''
{setup}
{key_setup}
_ziro_test_custom() {{ local -a descriptions=('alpha -- custom alpha' 'beta -- custom beta'); compadd -d descriptions -- alpha beta; }}
compdef _ziro_test_custom ziro-custom
_ziro_test_slow() {{ sleep 2; compadd late; }}
compdef _ziro_test_slow slowcmd
PROMPT='ZIRO> '
RPROMPT=''
source '{ROOT}/plugin/ziro.plugin.zsh'
functions[_ziro_test_original_bind]=$functions[_ziro_bind_all]
_ziro_bind_all() {{ if zle; then print violation > '{tmp}/bound-in-zle'; fi; _ziro_test_original_bind "$@"; }}
_ziro_test_snapshot() {{ print -rl -- "$BUFFER" "$CURSOR" "$_ziro_visible" "$_ziro_ghost" "$KEYMAP" > $ZIRO_TEST_STATE; }}
zle -N _ziro_test_snapshot
bindkey '^X^X' _ziro_test_snapshot
bindkey -M vicmd '^X^X' _ziro_test_snapshot
_ziro_test_cd() {{ builtin cd -- '{tmp}'; }}
zle -N _ziro_test_cd
bindkey '^Xc' _ziro_test_cd
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
        if os.environ.get('ZIRO_TEST_EXPLICIT')=='1' and text in ('\t','\x1b[B'):
            os.write(master,b'\0');drain(.05)
        os.write(master, text.encode()); drain(wait)
    def snapshot():
        send('\x18\x18', .2)
        return Path(env['ZIRO_TEST_STATE']).read_text().splitlines()
    try:
        deadline=time.monotonic()+15
        while b'ZIRO> ' not in output:
            assert time.monotonic()<deadline, 'shell did not reach first prompt'
            drain(.1)
        drain(.2)
        assert not Path(tmp+'/bound-in-zle').exists()
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
        # Nothing is highlighted before navigation: the first ↓ selects row 1,
        # the second moves to row 2 (sent raw so explicit mode keeps the menu active).
        send('\x1b[B', .2)
        os.write(master, b'\x1b[B'); drain(.2)
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
        send('cat ', .4)
        send('\x18c', .6)
        send('My', .4)
        assert snapshot()[2]=='1', snapshot()
        send('\t', .2)
        assert snapshot()[0]=='cat My\\ Docs/', snapshot()
        send('\x03', .2)
        send('echo history-one\r', .3)
        send('echo history-two\r', .3)
        send('\x1b[A', .3)
        # Do not run the snapshot widget between Up presses: that would
        # itself change LASTWIDGET and reset native prefix-search state.
        send('\x1b[A', .3)
        state=snapshot()
        assert state[0]=='echo history-one' and state[2]=='0', state
        send('\x03', .2)
        if os.environ.get('ZIRO_TEST_VI')=='1':
            send('zzzznosuggestions', .2)
            send('\x1b', .6)
            send('0', .2)
            state=snapshot()
            assert state[0]=='zzzznosuggestions' and state[1]=='0', state
        if os.environ.get('ZIRO_TEST_RESTART')=='1':
            send('\x03', .2)
            daemon.kill();daemon.wait(timeout=2)
            send('echo offline', .2)
            assert snapshot()[0]=='echo offline'
            send('\x03', .5)
            send('ziro-custom ', .6)
            assert snapshot()[2]=='1', snapshot()
            send('\x03', .2)
        assert not Path(tmp+'/bound-in-zle').exists()
        assert b'maximum nested function level' not in output, 'recursive plugin widget wrapper'
        if os.environ.get('ZIRO_TEST_LIFECYCLE')=='1':
            for _ in range(20): send('\x03', .05)
            drain(.2)
            children=Path(f'/proc/{shell.pid}/task/{shell.pid}/children').read_text().split()
            workers=[]
            for child in children:
                try:
                    command=Path(f'/proc/{child}/cmdline').read_bytes().split(b'\0')[0]
                    if command.endswith(b'zsh'):workers.append(child)
                except FileNotFoundError:pass
            assert len(workers)<=1, workers
        print('PASS: widget cwd restart; menu, Tab, Enter, history, ghost, Ctrl+C, custom compdef, spaces/quotes/Unicode, glob fallback, slow worker isolation/recovery')
    except BaseException:
        print(output.decode(errors='replace')[-1000:])
        raise
    finally:
        shell.terminate()
        try: shell.wait(timeout=2)
        except subprocess.TimeoutExpired: shell.kill(); shell.wait()
        if os.environ.get('ZIRO_TEST_TMUX')=='1':
            subprocess.run(['tmux','-L',tmux_socket,'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        if daemon.poll() is None:
            daemon.terminate(); daemon.wait(timeout=2)
        elif os.environ.get('ZIRO_TEST_RESTART')=='1':
            lock=Path(tmp+'/daemon.lock')
            if lock.exists():
                pid=int(lock.read_text().strip())
                if pid!=daemon.pid:
                    try:os.kill(pid,signal.SIGTERM)
                    except ProcessLookupError:pass
        errors=daemon.stderr.read().decode()
        if errors: print(errors)
        os.close(master)

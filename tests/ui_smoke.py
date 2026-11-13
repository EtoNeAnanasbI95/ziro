#!/usr/bin/env python3
"""Real ZLE regression checks for directory suggestions and frame stability."""
import fcntl
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='ziro-ui-') as tmp:
    temp = Path(tmp)
    work = temp / 'work'
    work.mkdir()
    (work / 'Alpha Space' / 'Nested').mkdir(parents=True)
    (work / 'Beta').mkdir()
    (work / 'Single' / 'Only').mkdir(parents=True)
    (work / 'plain.txt').write_text('file')
    env = os.environ | {
        'ZDOTDIR': tmp, 'XDG_STATE_HOME': tmp + '/state',
        'XDG_CONFIG_HOME': tmp + '/config', 'ZIRO_SOCKET': tmp + '/daemon.sock',
        'ZIRO_BIN': str(ROOT / 'target/debug/ziro'), 'TERM': 'xterm-256color',
        'ZIRO_TEST_STATE': tmp + '/editor',
    }
    env.pop('ISTERM', None)
    (temp / '.zshrc').write_text(f'''
autoload -Uz compinit
compinit -D
bindkey -e
PROMPT='UI > '
RPROMPT=''
alias uialias='print alias'
uifunction() {{ :; }}
_uislow() {{ sleep 2; compadd late; }}
compdef _uislow slowui
builtin cd -- '{work}'
source '{ROOT}/plugin/ziro.plugin.zsh'
_ui_snapshot() {{ print -rn -- "$BUFFER"$'\\0'"$CURSOR"$'\\0'"$_ziro_visible"$'\\0'"$POSTDISPLAY"$'\\0'"$_ziro_gen"$'\\0'"${{_ziro_pty_fd:-}}" > $ZIRO_TEST_STATE; }}
zle -N _ui_snapshot
bindkey '^X^X' _ui_snapshot
''')
    daemon = subprocess.Popen([env['ZIRO_BIN'], 'daemon', '--socket', env['ZIRO_SOCKET']], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 100, 0, 0))
    def attach():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
    shell = subprocess.Popen(['zsh', '-i'], stdin=slave, stdout=slave, stderr=slave, env=env, preexec_fn=attach)
    os.close(slave)
    output = bytearray()
    def drain(seconds):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], min(.02, max(0, end - time.monotonic())))[0]:
                try: output.extend(os.read(master, 65536))
                except OSError: break
    def send(text, delay=.15):
        os.write(master, text.encode())
        drain(delay)
    def snapshot():
        send('\x18\x18', .1)
        return Path(env['ZIRO_TEST_STATE']).read_bytes().decode().split('\0')
    def reset(): send('\x03', .25)
    try:
        deadline=time.monotonic()+10
        while b'UI > ' not in output:
            assert time.monotonic()<deadline
            drain(.1)
        drain(.2)
        for char in 'cd ': send(char, .15)
        state=snapshot()
        assert state[2]=='1' and 'Alpha Space' in state[3], ('cd empty', state)
        send('Al', .4)
        state=snapshot()
        assert state[2]=='1' and 'Alpha Space' in state[3], ('cd prefix', state)
        send('\t', .25)
        assert snapshot()[0]=='cd Alpha\\ Space/', snapshot()
        state=snapshot()
        assert state[2]=='1' and 'Nested' in state[3], ('nested after Tab', state)
        reset()
        send('cd Single/', .5)
        state=snapshot()
        assert state[2]=='1' and 'Only' in state[3], ('single directory', state)
        send('\t', .2)
        assert snapshot()[0]=='cd Single/Only/', snapshot()
        reset()
        send('cd '+str(work)+'/Al', .4)
        state=snapshot()
        assert state[2]=='1' and 'Alpha Space' in state[3], ('absolute', state)
        reset()
        send('cd ~/De', .5)
        state=snapshot()
        assert state[2]=='1' and 'Desktop' in state[3], ('home path', state)
        reset()
        send('slowui ', .8)
        send('\x15', .1)
        send('cd ', .8)
        state=snapshot()
        assert state[2]=='1' and 'Alpha Space' in state[3], ('cd after worker timeout without Enter', state)
        for prefix, emoji, label in [('uial','🔗','alias'),('uifu','🧩','function'),('pri','🐚','builtin'),('zs','📦','binary')]:
            reset()
            send(prefix, .6)
            state=snapshot()
            assert state[2]=='1' and emoji in state[3] and label in state[3], ('command identity', prefix, state)
        print('PASS: command identity emojis; cd empty, slow typing, prefix, escaped path, next directory after Tab, single directory, absolute, home')
    except BaseException:
        print(output.decode(errors='replace')[-3500:])
        raise
    finally:
        shell.terminate()
        try: shell.wait(timeout=2)
        except subprocess.TimeoutExpired: shell.kill(); shell.wait()
        daemon.terminate(); daemon.wait(timeout=2)
        errors=daemon.stderr.read().decode()
        if errors: print(errors)
        os.close(master)

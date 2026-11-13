#!/usr/bin/env python3
"""Exercise inherited compsys through a single zpty worker and real IPC."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
US = '\x1f'
with tempfile.TemporaryDirectory(prefix='ziro-worker-') as tmp:
    sock = socket.socket(socket.AF_UNIX)
    sock.bind(tmp + '/probe.sock')
    sock.listen()
    sock.settimeout(10)
    script = '''
emulate -R zsh
zmodload zsh/zpty
zmodload zsh/net/socket
zmodload zsh/system
zmodload zsh/datetime
autoload -Uz compinit
compinit -D
_ziro_test_completion() { local -a ds=('alpha -- inherited alpha' 'beta -- inherited beta'); compadd -d ds -- alpha beta; }
compdef _ziro_test_completion ziro-test
_ziro_us=$'\\x1f'
_ziro_session=probe
_ziro_socket=$1
source "$2/plugin/lib/worker.zsh"
zpty -b ziro-worker _ziro_worker_main
fd=$REPLY
while zpty -t ziro-worker; do
  chunk=''
  zpty -r ziro-worker chunk && print -rn -- "$chunk"
  sleep 0.001
done
'''
    output_log = tempfile.TemporaryFile()
    proc = subprocess.Popen(['zsh', '-f', '-c', script, 'probe', tmp + '/probe.sock', str(ROOT)], stdout=output_log, stderr=subprocess.STDOUT)
    conn, _ = sock.accept()
    conn.settimeout(5)
    tail = b''
    def record():
        global tail
        while b'\0' not in tail:
            chunk = conn.recv(65536)
            if not chunk:
                raise RuntimeError('worker disconnected')
            tail += chunk
        raw, tail = tail.split(b'\0', 1)
        return raw.decode().split(US)
    try:
        assert record() == ['WORKER_HELLO', 'probe', str(ROOT)]
        results = []
        for generation, query in enumerate(['git ch', 'git checkout ', 'tar -', 'ls ~/De', 'ziro-test '], 1):
            begin = time.monotonic()
            conn.sendall(US.join(['REQ', str(generation), str(len(query)), query]).encode() + b'\0')
            records = []
            while True:
                fields = record()
                records.append(fields)
                if fields[0] == 'END':
                    break
            candidates = [f for f in records if f[0] == 'C']
            assert candidates, (query, records)
            if query == 'ziro-test ':
                assert any('inherited alpha' in US.join(f) for f in candidates), candidates
            results.append({'query': query, 'candidates': len(candidates), 'ms': round((time.monotonic()-begin)*1000, 3), 'context': records[0], 'examples': candidates[:3]})
        if os.environ.get('ZIRO_STRESS') == '1':
            begin=time.monotonic()
            for generation in range(6, 2006):
                conn.sendall(US.join(['REQ', str(generation), '10', 'ziro-test ']).encode()+b'\0')
                while record()[0] != 'END':
                    pass
            results.append({'stress_requests':2000,'seconds':round(time.monotonic()-begin,3)})
        print(json.dumps(results, ensure_ascii=False, indent=2))
    finally:
        conn.close()
        proc.terminate()
        try:
            proc.communicate(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.communicate()
        output_log.seek(0)
        output=output_log.read()
        output_log.close()
        if proc.returncode not in (0, -15):
            print(output.decode(errors='replace'))

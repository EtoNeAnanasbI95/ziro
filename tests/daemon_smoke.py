#!/usr/bin/env python3
"""IPC contracts against the real daemon with fake shell and worker peers."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
ROOT=Path(__file__).resolve().parents[1]
US='\x1f'
class Peer:
    def __init__(self,path):
        self.socket=socket.socket(socket.AF_UNIX); self.socket.settimeout(2); self.socket.connect(path);self.tail=b''
    def send(self,*fields):self.socket.sendall(US.join(map(str,fields)).encode()+b'\0')
    def read(self):
        while b'\0' not in self.tail:
            data=self.socket.recv(65536)
            assert data, 'EOF'
            self.tail+=data
        data,self.tail=self.tail.split(b'\0',1)
        return data.decode().split(US)
    def until(self,kind,generation=None):
        for _ in range(30):
            m=self.read()
            if m[0]==kind and (generation is None or m[1]==str(generation)):return m
        raise AssertionError(kind)
    def ctx(self,generation,buffer):self.send('CTX',generation,len(buffer),80,24,'emacs','/tmp','',buffer)
    def batch(self,generation,words):
        self.send('CANDS',generation,'','','','','',len(words))
        for word in words:self.send('C','group','commands',word,word+' -- description','','','','')
        self.send('END',generation)
with tempfile.TemporaryDirectory(prefix='ziro-daemon-') as tmp:
    env=os.environ|{'XDG_STATE_HOME':tmp+'/state','XDG_CONFIG_HOME':tmp+'/config'}
    Path(tmp+'/config/ziro').mkdir(parents=True)
    Path(tmp+'/config/ziro/config.toml').write_text('[worker]\nrefresh_idle_ms=10000\ntimeout_ms=2000\n')
    path=tmp+'/daemon.sock'
    proc=subprocess.Popen([str(ROOT/'target/debug/ziro'),'daemon','--socket',path],env=env,stderr=subprocess.PIPE)
    try:
        for _ in range(100):
            if Path(path).exists():break
            time.sleep(.01)
        s=Peer(path);s.send('HELLO','v1','test',os.getpid(),'tty','xterm','5.9');assert s.read()[0]=='CFG'
        w=Peer(path);w.send('WORKER_HELLO','test','/tmp')
        s.ctx(1,'git ch');assert w.until('REQ',1)[3]=='git ch';w.batch(1,['checkout','cherry-pick']);frame=s.until('FRAME',1);assert 'checkout' in frame[5]
        s.ctx(2,'git che');assert 'checkout' in s.until('FRAME',2)[5]
        s.ctx(3,'ls ');assert w.until('REQ',3)[3]=='ls '
        s.ctx(4,'tar -');s.ctx(5,'cat ')
        w.batch(3,['stale']);assert w.until('REQ',5)[3]=='cat '
        w.batch(5,['file']);assert 'file' in s.until('FRAME',5)[5]
        s.send('HIST_START','event',time.time(),'/tmp','echo hello');s.send('HIST_END','event',0,10)
        s.ctx(6,'echo h');assert 'ello' in s.until('FRAME',6)[5]
        bad=Peer(path);bad.send('HELLO','v99','bad',0,'','','');assert bad.read()[0:2]==['ERR','proto'];bad.socket.close()
        assert Path(path).stat().st_mode & 0o777==0o600
        s.send('BYE');s.socket.close();w.socket.close();time.sleep(.05)
        stats=json.loads(Path(tmp+'/state/ziro/stats.json').read_text());assert stats['history_entries']==1
        print('PASS: CTX→REQ→CANDS→FRAME, cache, stale generation, coalescing, history, version, permissions')
    finally:
        proc.terminate();proc.wait(timeout=2)
        errors=proc.stderr.read().decode()
        if errors:print(errors)

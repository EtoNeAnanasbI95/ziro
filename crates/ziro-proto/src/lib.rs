//! Protocol v1: UTF-8 fields separated by US, NUL-terminated records.
use std::{error::Error, fmt};
pub const US: char = '\x1f';
pub const MAX_MESSAGE: usize = 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError(pub String);
impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl Error for ProtocolError {}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub kind: String,
    pub fields: Vec<String>,
}
impl Message {
    pub fn new(kind: &str, fields: impl IntoIterator<Item = impl ToString>) -> Self {
        Self {
            kind: kind.into(),
            fields: fields.into_iter().map(|v| v.to_string()).collect(),
        }
    }
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut fields = vec![self.kind.as_str()];
        fields.extend(self.fields.iter().map(String::as_str));
        if fields.iter().any(|s| s.contains(['\0', US, '\x1e'])) {
            return Err(ProtocolError("reserved character in field".into()));
        }
        let mut bytes = fields.join("\x1f").into_bytes();
        if bytes.len() > MAX_MESSAGE {
            return Err(ProtocolError("message exceeds 1 MiB".into()));
        }
        bytes.push(0);
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > MAX_MESSAGE || bytes.contains(&0) || bytes.contains(&0x1e) {
            return Err(ProtocolError("invalid record".into()));
        }
        let text = std::str::from_utf8(bytes).map_err(|e| ProtocolError(e.to_string()))?;
        let mut fields = text.split(US);
        let kind = fields.next().unwrap_or_default().to_owned();
        let fields: Vec<String> = fields.map(str::to_owned).collect();
        let count = fields.len();
        let valid = match kind.as_str() {
            "HELLO" => count == 6,
            "CTX" => count == 8,
            "KEY" => count == 2,
            "HIST_START" => count == 4,
            "HIST_END" => count == 3,
            "TRACE" => count >= 1,
            "BYE" => count == 0,
            "WORKER_HELLO" => count == 2,
            "CANDS" => count == 7,
            "C" => count == 8,
            "END" => count == 1,
            "REQ" => count == 3,
            "CFG" => count >= 3,
            "FRAME" => count >= 6,
            "CLEAR" | "WAIT" => count == 1,
            "ERR" => count == 2,
            "UI_STATE" | "UI_CLEAR" => count == 1,
            "UI_SELECT" => count == 2,
            _ => false,
        };
        if !valid {
            return Err(ProtocolError(format!("invalid {kind} field count {count}")));
        }
        let message = Self { kind, fields };
        message.validate()?;
        Ok(message)
    }
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let numeric: &[usize] = match self.kind.as_str() {
            "HELLO" => &[2],
            "CTX" => &[0, 1, 2, 3],
            "KEY" | "CLEAR" | "WAIT" | "END" | "UI_CLEAR" | "UI_STATE" => &[0],
            "HIST_END" => &[2],
            "REQ" | "UI_SELECT" => &[0, 1],
            "CANDS" => &[0, 6],
            "CFG" => &[2],
            "FRAME" => &[0, 1, 2, 3, 5],
            "TRACE" => &[0],
            _ => &[],
        };
        for &index in numeric {
            self.number::<u64>(index)?;
        }
        match self.kind.as_str() {
            "FRAME" => {
                let rows = self.number::<usize>(3)?;
                let hl = self.number::<usize>(5)?;
                let expected = rows
                    .checked_mul(4)
                    .and_then(|n| n.checked_add(hl))
                    .and_then(|n| n.checked_add(6));
                if expected != Some(self.fields.len()) {
                    return Err(ProtocolError("invalid FRAME shape".into()));
                }
                if rows > 0 && self.number::<usize>(2)? >= rows {
                    return Err(ProtocolError("selected outside FRAME".into()));
                }
                for i in 0..rows {
                    for col in [0, 1, 3] {
                        self.number::<usize>(6 + hl + i * 4 + col)?;
                    }
                }
            }
            "TRACE" => {
                if self.number::<usize>(0)?.checked_add(1) != Some(self.fields.len()) {
                    return Err(ProtocolError("invalid TRACE count".into()));
                }
                for f in self.fields.iter().skip(1) {
                    f.parse::<u64>()
                        .map_err(|_| ProtocolError("invalid TRACE sample".into()))?;
                }
            }
            "KEY" => {
                if !["next", "prev", "accept", "dismiss", "page_up", "page_down"]
                    .contains(&self.field(1))
                {
                    return Err(ProtocolError("unknown key action".into()));
                }
            }
            "HIST_START" => {
                let ts = self.number::<f64>(1)?;
                if !ts.is_finite() {
                    return Err(ProtocolError("invalid timestamp".into()));
                }
            }
            "HIST_END" => {
                self.number::<i32>(1)?;
            }
            _ => {}
        }
        Ok(())
    }
    pub fn field(&self, n: usize) -> &str {
        self.fields.get(n).map(String::as_str).unwrap_or("")
    }
    pub fn number<T: std::str::FromStr>(&self, n: usize) -> Result<T, ProtocolError> {
        self.field(n)
            .parse()
            .map_err(|_| ProtocolError(format!("invalid numeric field {n}")))
    }
}
// Typed direction envelopes prevent mixing shell, worker and daemon records.
macro_rules! messages {
    ($name:ident { $($variant:ident => $kind:literal),* $(,)? }) => {
        #[derive(Debug,Clone,PartialEq,Eq)]
        pub enum $name { $($variant(Message)),* }
        impl TryFrom<Message> for $name {
            type Error=ProtocolError;
            fn try_from(m:Message)->Result<Self,ProtocolError>{m.validate()?;match m.kind.as_str(){$($kind=>Ok(Self::$variant(m)),)*_=>Err(ProtocolError("wrong message direction".into()))}}
        }
        impl From<$name> for Message {fn from(m:$name)->Self{match m{$($name::$variant(m)=>m),*}}}
    }
}
messages!(ShellMsg{Hello=>"HELLO",Ctx=>"CTX",Key=>"KEY",HistStart=>"HIST_START",HistEnd=>"HIST_END",Trace=>"TRACE",Bye=>"BYE"});
messages!(WorkerMsg{Hello=>"WORKER_HELLO",Candidates=>"CANDS",Candidate=>"C",End=>"END"});
messages!(DaemonMsg{Request=>"REQ",Config=>"CFG",Frame=>"FRAME",Clear=>"CLEAR",Wait=>"WAIT",Error=>"ERR"});
#[derive(Default)]
pub struct Framer {
    tail: Vec<u8>,
}
impl Framer {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Message>, ProtocolError> {
        let mut result = Vec::new();
        for &byte in bytes {
            if byte == 0 {
                let record = std::mem::take(&mut self.tail);
                result.push(Message::decode(&record)?);
            } else {
                if self.tail.len() >= MAX_MESSAGE {
                    self.tail.clear();
                    return Err(ProtocolError("message exceeds 1 MiB".into()));
                }
                self.tail.push(byte);
            }
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_fragmentation() {
        let examples = [
            "HELLO|v1|session|123|tty|xterm|5.9",
            "CTX|1|4|80|24|emacs|/tmp||файл\n",
            "KEY|1|next",
            "HIST_START|id|123.5|/tmp|echo привет\n",
            "HIST_END|id|1|123",
            "TRACE|2|12|34",
            "BYE",
            "WORKER_HELLO|session|/tmp",
            "CANDS|1|p|s|ip|is||1",
            "C|g|t|файл|файл -- описание|||P|S",
            "END|1",
            "REQ|1|2|буфер",
            "CFG|menu|explicit|32|^I",
            "FRAME|1|0|0|1|меню|1|0 4 fg=8|0|4|git checkout |13",
            "CLEAR|1",
            "WAIT|1",
            "ERR|proto|wrong version",
        ];
        for example in examples {
            let raw = example.replace('|', "\x1f");
            let msg = Message::decode(raw.as_bytes()).unwrap();
            let mut f = Framer::default();
            let mut out = vec![];
            for b in msg.encode().unwrap() {
                out.extend(f.push(&[b]).unwrap());
            }
            assert_eq!(out, vec![msg]);
        }
    }
    #[test]
    fn exact_frame_bytes() {
        let m = Message::new("FRAME", ["1", "0", "0", "0", "", "0"]);
        assert_eq!(m.encode().unwrap(), b"FRAME\x1f1\x1f0\x1f0\x1f0\x1f\x1f0\0");
    }
    #[test]
    fn invalid_frame_and_numbers() {
        for raw in [
            "FRAME|1|0|0|1||0",
            "CTX|bad|0|80|24|main|/tmp||",
            "KEY|1|oops",
            "TRACE|10|1",
            "HIST_START|id|NaN|/tmp|cmd",
        ] {
            assert!(Message::decode(raw.replace('|', "\x1f").as_bytes()).is_err());
        }
    }
    #[test]
    fn rejects_reserved_and_oversize() {
        assert!(Message::new("ERR", ["bad\x1f", "x"]).encode().is_err());
        assert!(
            Framer::default()
                .push(&vec![b'x'; MAX_MESSAGE + 1])
                .is_err()
        );
        assert!(Message::decode(b"WHAT").is_err());
    }
}

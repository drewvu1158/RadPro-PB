//! The AMF3 subset used by Sparkbooth v2 SBL layout documents.
//! Reference tables are shared by the whole document, including embedded art.
use std::{collections::BTreeMap, sync::Arc};

pub const MAX_FILE: usize = 32 * 1024 * 1024;
pub type Object = BTreeMap<String, Value>;

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(Arc<str>),
    Bytes(Arc<[u8]>),
    Array(Arc<Vec<Value>>),
    Object(Arc<Object>),
}

#[derive(Clone)]
struct Traits {
    dynamic: bool,
    names: Vec<String>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    nodes: usize,
    strings: Vec<Arc<str>>,
    objects: Vec<Option<Value>>,
    traits: Vec<Traits>,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.pos.checked_add(n).ok_or("AMF size overflow")?;
        let b = self
            .bytes
            .get(self.pos..end)
            .ok_or("Truncated SBL document")?;
        self.pos = end;
        Ok(b)
    }
    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u29(&mut self) -> Result<u32, String> {
        let mut v = 0;
        for _ in 0..3 {
            let b = self.byte()?;
            if b < 128 {
                return Ok((v << 7) | u32::from(b));
            }
            v = (v << 7) | u32::from(b & 127);
        }
        Ok((v << 8) | u32::from(self.byte()?))
    }
    fn string(&mut self) -> Result<Arc<str>, String> {
        let h = self.u29()?;
        if h & 1 == 0 {
            return self
                .strings
                .get((h >> 1) as usize)
                .cloned()
                .ok_or("Invalid AMF string reference".into());
        }
        if h == 1 {
            return Ok(Arc::from(""));
        }
        let s: Arc<str> = Arc::from(
            std::str::from_utf8(self.take((h >> 1) as usize)?)
                .map_err(|_| "Invalid UTF-8 in SBL")?,
        );
        self.strings.push(s.clone());
        Ok(s)
    }
    fn value(&mut self, depth: usize) -> Result<Value, String> {
        self.nodes += 1;
        if depth > 64 || self.nodes > 20_000 {
            return Err("SBL exceeds the supported document complexity".into());
        }
        let marker = self.byte()?;
        match marker {
            0 | 1 => Ok(Value::Null),
            2 | 3 => Ok(Value::Bool(marker == 3)),
            4 => {
                let u = self.u29()?;
                let n = if u & 0x10000000 != 0 {
                    i64::from(u) - 0x20000000
                } else {
                    i64::from(u)
                };
                Ok(Value::Number(n as f64))
            }
            5 => Ok(Value::Number(f64::from_be_bytes(
                self.take(8)?.try_into().unwrap(),
            ))),
            6 => Ok(Value::String(self.string()?)),
            9 | 10 | 12 => {
                let h = self.u29()?;
                if h & 1 == 0 {
                    return self
                        .objects
                        .get((h >> 1) as usize)
                        .and_then(Clone::clone)
                        .ok_or("Invalid or cyclic AMF object reference".into());
                }
                let reference = self.objects.len();
                self.objects.push(None);
                let value = match marker {
                    12 => Value::Bytes(Arc::from(self.take((h >> 1) as usize)?)),
                    9 => {
                        if h >> 1 > 20_000 {
                            return Err("Too many SBL array entries".into());
                        }
                        if !self.string()?.is_empty() {
                            return Err(
                                "Associative AMF arrays are not supported in SBL layouts".into()
                            );
                        }
                        let mut items = Vec::new();
                        for _ in 0..h >> 1 {
                            items.push(self.value(depth + 1)?);
                        }
                        Value::Array(Arc::new(items))
                    }
                    _ => {
                        let traits = if h & 2 == 0 {
                            self.traits
                                .get((h >> 2) as usize)
                                .cloned()
                                .ok_or("Invalid AMF trait reference")?
                        } else {
                            if h & 4 != 0 {
                                return Err("Externalizable SBL objects are not supported".into());
                            }
                            let class = self.string()?;
                            if !class.is_empty() {
                                return Err(format!("Unsupported SBL object class: {class}"));
                            }
                            if h >> 4 > 20_000 {
                                return Err("Too many SBL object fields".into());
                            }
                            let mut names = Vec::new();
                            for _ in 0..h >> 4 {
                                names.push(self.string()?.to_string());
                            }
                            let t = Traits {
                                dynamic: h & 8 != 0,
                                names,
                            };
                            self.traits.push(t.clone());
                            t
                        };
                        let mut fields = Object::new();
                        for name in traits.names {
                            if fields.insert(name, self.value(depth + 1)?).is_some() {
                                return Err("Duplicate SBL field".into());
                            }
                        }
                        if traits.dynamic {
                            loop {
                                let name = self.string()?;
                                if name.is_empty() {
                                    break;
                                }
                                if fields
                                    .insert(name.to_string(), self.value(depth + 1)?)
                                    .is_some()
                                {
                                    return Err("Duplicate SBL field".into());
                                }
                            }
                        }
                        Value::Object(Arc::new(fields))
                    }
                };
                self.objects[reference] = Some(value.clone());
                Ok(value)
            }
            _ => Err(format!(
                "Unsupported AMF3 marker 0x{marker:02x} at byte {}",
                self.pos - 1
            )),
        }
    }
}

pub fn decode(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > MAX_FILE {
        return Err("SBL exceeds the 32 MiB import memory budget".into());
    }
    let mut reader = Reader {
        bytes,
        pos: 0,
        nodes: 0,
        strings: vec![],
        objects: vec![],
        traits: vec![],
    };
    let value = reader.value(0)?;
    if reader.pos != bytes.len() {
        return Err("Unexpected trailing data in SBL document".into());
    }
    Ok(value)
}

fn u29(out: &mut Vec<u8>, v: u32) {
    if v < 0x80 {
        out.push(v as u8);
    } else if v < 0x4000 {
        out.extend([(v >> 7) as u8 | 128, (v & 127) as u8]);
    } else if v < 0x200000 {
        out.extend([
            (v >> 14) as u8 | 128,
            ((v >> 7) & 127) as u8 | 128,
            (v & 127) as u8,
        ]);
    } else {
        out.extend([
            (v >> 22) as u8 | 128,
            ((v >> 15) & 127) as u8 | 128,
            ((v >> 8) & 127) as u8 | 128,
            v as u8,
        ]);
    }
}
fn string(out: &mut Vec<u8>, s: &str) {
    u29(out, ((s.len() as u32) << 1) | 1);
    out.extend(s.as_bytes());
}

pub fn encode(value: &Value) -> Result<Vec<u8>, String> {
    fn write(out: &mut Vec<u8>, v: &Value, depth: usize) -> Result<(), String> {
        if depth > 64 {
            return Err("SBL nesting limit exceeded".into());
        }
        let extra = match v {
            Value::Bytes(b) => b.len(),
            Value::String(s) => s.len(),
            _ => 16,
        };
        if out.len().saturating_add(extra).saturating_add(16) > MAX_FILE {
            return Err("SBL exceeds the 32 MiB export memory budget".into());
        }
        match v {
            Value::Null => out.push(1),
            Value::Bool(b) => out.push(if *b { 3 } else { 2 }),
            Value::Number(n) => {
                out.push(5);
                out.extend(n.to_be_bytes());
            }
            Value::String(s) => {
                out.push(6);
                string(out, s);
            }
            Value::Bytes(b) => {
                out.push(12);
                u29(out, ((b.len() as u32) << 1) | 1);
                out.extend(b.iter());
            }
            Value::Array(items) => {
                out.push(9);
                u29(out, ((items.len() as u32) << 1) | 1);
                out.push(1);
                for item in items.iter() {
                    write(out, item, depth + 1)?;
                }
            }
            Value::Object(fields) => {
                out.extend([10, 11, 1]);
                for (key, val) in fields.iter() {
                    if out.len().saturating_add(key.len()).saturating_add(16) > MAX_FILE {
                        return Err("SBL export exceeds memory budget".into());
                    }
                    string(out, key);
                    write(out, val, depth + 1)?;
                }
                out.push(1);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    write(&mut out, value, 0)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_and_malformed_boundaries() {
        // Dense array with a string followed by a reference to the same string.
        let Value::Array(a) = decode(&[9, 5, 1, 6, 7, b'a', b'b', b'c', 6, 0]).unwrap() else {
            panic!()
        };
        assert!(matches!(&a[1], Value::String(s) if s.as_ref() == "abc"));
        assert!(decode(&[6, 0]).is_err());
        assert!(decode(&[12, 9, 1]).is_err());
        assert!(decode(&[10, 11, 1, 3, b'x', 10, 0, 1]).is_err()); // cycle
        assert!(decode(&[1, 1]).is_err());
        let Value::Number(n) = decode(&[4, 255, 255, 255, 255]).unwrap() else {
            panic!()
        };
        assert_eq!(n, -1.0);
    }
}

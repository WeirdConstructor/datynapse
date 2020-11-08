use std::fmt::Write;

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Direct(Vec<String>),
    Payload(&'static str, Option<Vec<String>>, Vec<u8>),
    Hello(Vec<String>),
    Error(Vec<String>),
    Ping,
    Quit,
    Ok(String),
}

fn write_msg_arg(w: &mut dyn std::fmt::Write, arg: &str) {
    if let Some(_) = arg.find(|c: char| c.is_whitespace() || c == '"') {
        write!(w, "\"").expect("write to work");
        for c in arg.chars() {
            match c {
                '"'  => write!(w, "\\\"").expect("write to work"),
                '\\' => write!(w, "\\\\").expect("write to work"),
                '\r' => write!(w, "\\r").expect("write to work"),
                '\n' => write!(w, "\\n").expect("write to work"),
                c    => w.write_char(c).expect("write to work"),
            }
        }
        write!(w, "\"").expect("write to work");
    } else {
        write!(w, "{}", arg).expect("write to work");
    }
}

fn write_msg_args(w: &mut dyn std::fmt::Write, args: &[String]) {
    for a in args {
        write!(w, " ").expect("write to work");
        write_msg_arg(w, a);
    }
}

impl Msg {
    pub fn ok_response(&self) -> Option<Self> {
        match self {
            Msg::Ping                => Some(Msg::Ok("ping".to_string())),
            Msg::Quit                => Some(Msg::Ok("quit".to_string())),
            Msg::Ok(_arg)            => None,
            Msg::Hello(_args)        => Some(Msg::Ok("hello".to_string())),
            Msg::Error(_args)        => Some(Msg::Ok("error".to_string())),
            Msg::Direct(_args)       => Some(Msg::Ok("direct".to_string())),
            Msg::Payload(name, _, _) => Some(Msg::Ok(name.to_string())),
        }
    }

    pub fn to_frame(&self) -> Vec<u8> {
        let mut s = String::new();
        let mut payload : Option<&[u8]> = None;
        match self {
            Msg::Ping           => write!(&mut s, "ping").expect("write to work"),
            Msg::Quit           => write!(&mut s, "quit").expect("write to work"),
            Msg::Ok(arg)        => write!(&mut s, "ok {}", arg).expect("write to work"),
            Msg::Hello(args)    => {
                write!(&mut s, "hello").expect("write to work");
                write_msg_args(&mut s, args);
            },
            Msg::Error(args)    => {
                write!(&mut s, "error").expect("write to work");
                write_msg_args(&mut s, args);
            },
            Msg::Direct(args) => {
                write!(&mut s, "direct").expect("write to work");
                write_msg_args(&mut s, args);
            },
            Msg::Payload(name, args, msg_payload) => {
                write!(&mut s, "{}", name).expect("write to work");
                write!(&mut s, " {}", msg_payload.len()).expect("write to work");
                if let Some(args) = args {
                    write_msg_args(&mut s, args);
                }
                write!(&mut s, "\r\n").expect("write to work");
                payload = Some(&msg_payload[..]);
            },
        }

        write!(&mut s, "\r\n").expect("write to work");

        let mut b = s.into_bytes();
        if let Some(payload) = payload {
            b.extend_from_slice(payload);
            b.push(b'\r');
            b.push(b'\n');
        }

        b
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReadMsgError {
    TryAgain,
    ProtocolError,
    Timeout,
    EOF,
    ReadError(String, String),
}

fn next_arg(s: &str) -> (String, &str) {
    let s = s.trim_start();

    if let Some(s) = s.strip_prefix('"') {
        let mut arg = String::new();
        let mut escaped = false;
        for (byte_pos, c) in s.char_indices() {
            if escaped {
                match c {
                    'r' => arg.push('\r'),
                    'n' => arg.push('\n'),
                    _   => arg.push(c),
                }
                escaped = false;
            } else {
                if c == '"' {
                    return (arg, &s[(byte_pos + 1)..].trim_start());
                } else if c == '\\' {
                    escaped = true;
                } else {
                    arg.push(c);
                }
            }
        }

        (arg, "")

    } else {
        let v : Vec<&str> = s.splitn(2, |c: char| c.is_whitespace()).collect();
        if v.len() == 0 {
            ("".to_string(), "")
        } else if v.len() == 1 {
            (v[0].to_string(), "")
        } else {
            (v[0].to_string(), v[1].trim_start())
        }
    }
}

pub struct MessageReader {
    payload_name:       Option<&'static str>,
    payload_buf:        Option<Vec<u8>>,
    payload_rem_len:    Option<u64>,
    payload_data:       Option<Vec<String>>,
}

impl MessageReader {
    pub fn new() -> Self {
        Self {
            payload_name:       None,
            payload_buf:        None,
            payload_rem_len:    None,
            payload_data:       None,
        }
    }

    fn read_payload(&mut self, ep: &str, buf: &mut dyn std::io::BufRead) -> Result<Msg, ReadMsgError> {
        let rlen = self.payload_rem_len.unwrap();

        if rlen > 0 {
            let take_len = if rlen < 4096 { rlen as usize } else { 4096 };
            let mut data_buf : [u8; 4096] = [0; 4096];

            match buf.read(&mut data_buf[0..take_len]) {
                Ok(n) => {
                    if n == 0 {
                        return Err(ReadMsgError::EOF);
                    }
                    if n > take_len {
                        return Err(ReadMsgError::ProtocolError);
                    }

                    self.payload_rem_len = Some(rlen - (n as u64));
                    self.payload_buf.as_mut().unwrap().extend_from_slice(&data_buf[0..n]);
                },
                Err(e) => {
                    match e.kind() {
                        std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock => {
                            return Err(ReadMsgError::Timeout);
                        },
                        _ => {
                            return Err(ReadMsgError::ReadError(
                                ep.to_string(), format!("{}", e)));
                        }
                    }
                },
            }
        }

        if self.payload_rem_len.unwrap() == 0 {
            self.payload_rem_len = None;
            return Ok(Msg::Payload(
                self.payload_name.take().unwrap(),
                self.payload_data.take(),
                self.payload_buf.take().unwrap()));
        } else {
            return Err(ReadMsgError::TryAgain);
        }
    }

    pub fn read_msg(&mut self, ep: &str, buf: &mut dyn std::io::BufRead) -> Result<Msg, ReadMsgError> {
        if self.payload_rem_len.is_some() {
            return self.read_payload(ep, buf);
        }

        // use std::io::BufRead;
        let mut line = String::new();
        match buf.read_line(&mut line) {
            Ok(n) => {
                if n == 0 {
                    return Err(ReadMsgError::EOF);
                }
                //d// println!("READL: [{}]", line);

                let v: Vec<&str> =
                    line.splitn(2, |c: char| c.is_whitespace()).collect();

                let command  = v[0];
                let mut rest = if v.len() > 1 { v[1] } else { "" };

                match command {
                    "direct" => {
                        let mut args = vec![];
                        while rest.len() > 0 {
                            let (arg, r) = next_arg(rest);
                            rest = r;
                            args.push(arg);
                        }

                        return Ok(Msg::Direct(args));
                    },
                    "payload" => {
                        let (len_arg, r) = next_arg(rest);
                        rest = r;

                        let mut args = vec![];
                        while rest.len() > 0 {
                            let (arg, r) = next_arg(rest);
                            rest = r;
                            args.push(arg);
                        }

                        if let Ok(len) = u64::from_str_radix(&len_arg, 10) {
                            self.payload_rem_len  = Some(len);
                            self.payload_buf      = Some(vec![]);
                            self.payload_name     = Some("payload");
                            self.payload_data     = Some(args);
                            return self.read_payload(ep, buf);
                        } else {
                            return Err(ReadMsgError::ProtocolError);
                        }
                    },
                    "error" => {
                        let mut args = vec![];
                        while rest.len() > 0 {
                            let (arg, r) = next_arg(rest);
                            rest = r;
                            args.push(arg);
                        }

                        return Ok(Msg::Error(args));
                    },
                    "msgpack" => {
                        let (len_arg, _) = next_arg(rest);
                        if let Ok(len) = u64::from_str_radix(&len_arg, 10) {
                            self.payload_rem_len  = Some(len);
                            self.payload_buf      = Some(vec![]);
                            self.payload_name     = Some("msgpack");
                            return self.read_payload(ep, buf);
                        } else {
                            return Err(ReadMsgError::ProtocolError);
                        }
                    },
                    "json" => {
                        let (len_arg, _) = next_arg(rest);
                        if let Ok(len) = u64::from_str_radix(&len_arg, 10) {
                            self.payload_rem_len  = Some(len);
                            self.payload_buf      = Some(vec![]);
                            self.payload_name     = Some("json");
                            return self.read_payload(ep, buf);
                        } else {
                            return Err(ReadMsgError::ProtocolError);
                        }
                    },
                    "hello" => {
                        let mut args = vec![];
                        while rest.len() > 0 {
                            let (arg, r) = next_arg(rest);
                            rest = r;
                            args.push(arg);
                        }

                        return Ok(Msg::Hello(args));
                    },
                    "quit" => {
                        return Ok(Msg::Quit);
                    },
                    "ping" => {
                        return Ok(Msg::Ping);
                    },
                    "ok" => {
                        let (arg, _) = next_arg(rest);
                        return Ok(Msg::Ok(arg));
                    },
                    "" => {
                        return Err(ReadMsgError::TryAgain);
                    },
                    _ => {
                        return Err(ReadMsgError::ProtocolError);
                    },
                }
            },
            Err(e) => {
                match e.kind() {
                    std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::WouldBlock => {
                        return Err(ReadMsgError::Timeout);
                    },
                    _ => {
                        return Err(ReadMsgError::ReadError(
                            ep.to_string(), format!("{}", e)));
                    }
                }
            },
        }
    }
}

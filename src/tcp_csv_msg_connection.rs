use wlambda::VVal;
use std::sync::*;
use std::sync::atomic::AtomicBool;
use std::net::TcpStream;
use std::net::TcpListener;

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Direct(Vec<String>),
    Payload(&'static str, Vec<u8>),
    Hello(Vec<String>),
    Error(Vec<String>),
    Ping,
    Quit,
    Ok(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReadMsgError {
    TryAgain,
    NeedMoreData,
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
                arg.push(c);
                escaped = false;
            } else {
                if c == '"' {
                    return (arg, &s[(byte_pos + 1)..]);
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
            (v[0].to_string(), v[1])
        }
    }
}

pub struct MessageReader {
    payload_name: Option<&'static str>,
    payload_buf:  Option<Vec<u8>>,
    payload_len:  Option<u64>,
}

impl MessageReader {
    pub fn new() -> Self {
        Self {
            payload_name: None,
            payload_buf:  None,
            payload_len:  None,
        }
    }

    fn read_payload(&mut self, ep: &str, buf: &mut dyn std::io::BufRead) -> Result<Msg, ReadMsgError> {
        Err(ReadMsgError::ProtocolError)
    }

    pub fn read_msg(&mut self, ep: &str, buf: &mut dyn std::io::BufRead) -> Result<Msg, ReadMsgError> {
        if self.payload_len.is_some() {
            return self.read_payload(ep, buf);
        }

        use std::io::BufRead;
        let mut line = String::new();
        match buf.read_line(&mut line) {
            Ok(n) => {
                if n == 0 {
                    return Err(ReadMsgError::EOF);
                }

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
                            self.payload_len  = Some(len);
                            self.payload_buf  = Some(vec![]);
                            self.payload_name = Some("msgpack");
                            return self.read_payload(ep, buf);
                        } else {
                            return Err(ReadMsgError::ProtocolError);
                        }
                    },
                    "json" => {
                        let (len_arg, _) = next_arg(rest);
                        if let Ok(len) = u64::from_str_radix(&len_arg, 10) {
                            self.payload_len  = Some(len);
                            self.payload_buf  = Some(vec![]);
                            self.payload_name = Some("json");
                            return self.read_payload(ep, buf);
                        } else {
                            return Err(ReadMsgError::ProtocolError);
                        }
                    },
                    "hello" => {
                        return Ok(Msg::Hello(vec![]));
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


#[derive(Debug, Clone)]
pub enum Event {
    IncomingMessage(Msg),
    FlushedMessage,
    ConnectError(String),
    Connected(String),
    Quit,
}

type ClientID = usize;

#[derive(Debug, Clone)]
pub enum ServerEvent {
    Client(ClientID, Event),
    Server(Event),
}

pub struct TCPCSVServer {
    op_timeout_ms:      u64,
    connections:        Vec<Arc<TCPServerConnection>>,
    free_ids:           Vec<ClientID>,
}

pub struct TCPServerConInfo {
    last_activity_time: std::time::SystemTime,
}

pub struct TCPServerConnection {
    stream: TcpStream,
    info:   Mutex<TCPServerConInfo>,
}

impl TCPCSVServer {
    fn server_event(&self, e: Event) {
    }

    fn spawn_server_thread(&mut self) -> Result<(), std::io::Error> {
        let (stream_tx, stream_rx) = std::sync::mpsc::channel();
        let l = TcpListener::bind("0.0.0.0:57322")?;

        let op_timeout_ms = self.op_timeout_ms;
        std::thread::spawn(move || {
            for stream in l.incoming() {
                let stream =
                    match stream {
                        Ok(s) => s,
                        Err(e) => {
                            if let Err(e) =
                                stream_tx.send(
                                    Event::ConnectError(
                                        format!("Client listener error: {}", e)))
                            {
                                break;
                            }
                            continue;
                        }
                    };

                stream.set_nodelay(true)
                    .expect("Setting TCP no delay");
                stream.set_read_timeout(
                    Some(std::time::Duration::from_millis(op_timeout_ms / 10)))
                    .expect("Setting read timeout");
                stream.set_write_timeout(
                    Some(std::time::Duration::from_millis(op_timeout_ms / 10)))
                    .expect("Setting write timeout");
                stream.set_nonblocking(true)
                    .expect("Setting non-blocking");

            }
        });

        Ok(())
    }
}

fn set_stream_settings(stream: &mut TcpStream, op_timeout_ms: u64) {
    stream.set_nodelay(true)
        .expect("Setting TCP no delay");
    stream.set_read_timeout(
        Some(std::time::Duration::from_millis(op_timeout_ms / 10)))
        .expect("Setting read timeout");
    stream.set_write_timeout(
        Some(std::time::Duration::from_millis(op_timeout_ms / 10)))
        .expect("Setting write timeout");
}


pub struct TCPCSVConnection {
    op_timeout_ms:      u64,
    reader:             Option<std::thread::JoinHandle<()>>,
    writer_tx:          Option<mpsc::Sender<Option<Vec<u8>>>>,
    pub reader_rx:      Option<mpsc::Receiver<Event>>,
    stop:               std::sync::Arc<AtomicBool>,
}

impl TCPCSVConnection {
    pub fn new() -> Self {
        Self {
            op_timeout_ms: 10000,
            reader:        None,
            writer_tx:     None,
            reader_rx:     None,
            stop:          std::sync::Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn send(&mut self, s: &str) -> Result<(), std::sync::mpsc::SendError<Option<Vec<u8>>>> {
        if let Some(ref mut wtx) = self.writer_tx.as_mut() {
            wtx.send(Some(s.to_string().into_bytes()))?;
        }
        Ok(())
    }

//    fn spawn_writer_thread(mut writer_stream: TcpStream,
//                           wr_ep: String,
//                           writer_event_tx: mpsc::Sender<Event>,
//                           writer_rx: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<Option<Vec<u8>>>>>)
//        -> std::thread::JoinHandle<()>
//    {
//        std::thread::spawn(move || {
//            use std::io::Write;
//            match writer_rx.lock() {
//                Ok(wr_rx) => {
//                    while let Ok(Some(bytes)) = wr_rx.recv() {
//                        if let Err(e) = writer_stream.write_all(&bytes) {
//                            match e.kind() {
//                                std::io::ErrorKind::TimedOut
//                                | std::io::ErrorKind::WouldBlock => {
//                                    let Err(_) = writer_event_tx.send(
//                                        Event::ConnectError(
//                                            format!("Write error (timeout) from {}: {}",
//                                                    wr_ep, e))));
//                                    break;
//                                },
//                                _ => {
//                                    let Err(_) = writer_event_tx.send(
//                                        Event::ConnectError(
//                                            format!("Write error from {}: {}",
//                                                    wr_ep, e))));
//                                    break;
//                                }
//                            }
//                        }
//
//                        if let Err(e) = writer_stream.flush() {
//                            let Err(_) = writer_event_tx.send(
//                                Event::ConnectError(
//                                    format!("Write flush error from {}: {}",
//                                            wr_ep, e)));
//                            break;
//                        }
//
//                        let Err(_) = writer_event_tx.send(Event::FlushedMessage);
//                    }
//                },
//                Err(e) => {
//                    if let Err(_) = writer_event_tx.send(
//                        Event::ConnectError(
//                            format!("Write queue mutex error from {}: {}",
//                                    wr_ep, e))) {
//                    };
//                },
//            }
//            //d// eprintln!("writer ends");
//        })
//    }

    pub fn init_read_write(&mut self, mut stream: TcpStream) {
//        let (wr_tx, wr_rx)    = std::sync::mpsc::channel();
//        let (event_tx, rd_rx) = std::sync::mpsc::channel();
//
//        let wr_stop_tx = wr_tx.clone();
//
//        self.writer_tx = Some(wr_tx);
//        self.reader_rx = Some(rd_rx);
//
//        let wr_rx = std::sync::Arc::new(std::sync::Mutex::new(wr_rx));
//
//        let local_addr_str = match stream.local_addr() {
//            Ok(s)  => format!("{}", s),
//            Err(e) => format!("no-local-addr({})", e),
//        };
//
////        // clear write buffer
////        if let Ok(wrx) = wr_rx.lock() {
////            for _data in wrx.try_iter() {
////            }
////        }
//
//        let mut writer_stream =
//            stream
//            .try_clone()
//            .expect("Cloning stream should work for writer!");
//
//        let wr_ep           = ep.clone();
//        let writer_event_tx = event_tx.clone();
//        let writer_rx       = wr_rx.clone();
//        let writer =
//            spawn_writer_thread(
//                writer_stream, wr_ep, writer_event_tx, writer_rx);
//
//        if let Err(e) =
//            event_tx.send(
//                Event::Connected(
//                    format!("Connected to {} from {}",
//                            ep, local_addr_str))) {
//
//            wr_stop_tx.send(None);
//            writer.join();
//            return;
//        };
//
//        use std::io::BufRead;
//        let mut buf = std::io::BufReader::new(stream);
//        let stop_reader = self.stop.clone();
//        loop {
//            if stop_reader.load(std::sync::atomic::Ordering::Relaxed) {
//                break;
//            }
//
//            let mut line = String::new();
//            match buf.read_line(&mut line) {
//                Ok(n) => {
//                    if n == 0 {
//                        if let Err(_) = event_tx.send(
//                            Event::ConnectError(
//                                format!("EOF from {}", ep))) {
//                        };
//                        break;
//                    }
//
//                    if let Err(e) =
//                        event_tx.send(Event::IncomingMessage(line)) {
//                        break;
//                    };
//                },
//                Err(e) => {
//                    match e.kind() {
//                        std::io::ErrorKind::TimedOut
//                        | std::io::ErrorKind::WouldBlock => {
//                            continue;
//                        },
//                        _ => {
//                            if let Err(_) = event_tx.send(
//                                Event::ConnectError(
//                                    format!("Read error from {}: {}",
//                                            ep, e))) {
//                            };
//                            break;
//                        }
//                    }
//                },
//            }
//        }
//
//        wr_stop_tx.send(None);
//        writer.join();
    }

    pub fn connect(&mut self) {
        let op_timeout_ms = self.op_timeout_ms;

        self.stop.store(false, std::sync::atomic::Ordering::Relaxed);

        let ep = String::from("127.0.0.1:57322");
        let reader = std::thread::spawn(move || {
            loop {
                use std::str::FromStr;
                let mut stream =
                    TcpStream::connect_timeout(
                        &std::net::SocketAddr::from_str(&ep)
                         .expect("Valid endpoint address"),
                        std::time::Duration::from_millis(op_timeout_ms));

                match stream {
                    Ok(mut stream) => {
                        set_stream_settings(&mut stream, op_timeout_ms);
//                        self.init_read_write(stream);
                    },
                    Err(err) => {
//                        if let Err(e) =
////                            event_tx.send(
////                                Event::ConnectError(
////                                    format!("Couldn't connect to {}: {:?}", ep, err))) {
//                            break;
//                        }
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        continue;
                    },
                }

                //d// eprintln!("reader ends (2)");
            }
        });

        std::mem::replace(&mut self.reader, Some(reader));
    }

    pub fn shutdown(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(wtx) = &self.writer_tx {
            if let Err(_) = wtx.send(None) {
            }
        }
        std::mem::replace(&mut self.reader_rx, None);
        std::mem::replace(&mut self.writer_tx, None);
        let rd = std::mem::replace(&mut self.reader, None);
        if let Some(rd) = rd {
            if let Err(_) = rd.join() {
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_arg_parsing() {
        let (arg, rest) = next_arg("foo bar exop");
        assert_eq!(arg, "foo");
        assert_eq!(rest, "bar exop");

        let (arg, rest) = next_arg("foo");
        assert_eq!(arg, "foo");
        assert_eq!(rest, "");

        let (arg, rest) = next_arg("");
        assert_eq!(arg, "");
        assert_eq!(rest, "");

        let (arg, rest) = next_arg("\"foo\" bar exop");
        assert_eq!(arg, "foo");
        assert_eq!(rest, " bar exop");

        let (arg, rest) = next_arg("\"fo\\\"o\" \"bar oo\" exop");
        assert_eq!(arg, "fo\"o");
        assert_eq!(rest, " \"bar oo\" exop");

        let (arg, rest) = next_arg(rest);
        assert_eq!(arg, "bar oo");
        assert_eq!(rest, " exop");

        let (arg, rest) = next_arg(rest);
        assert_eq!(arg, "exop");
        assert_eq!(rest, "");
    }

    #[test]
    fn check_read_msg() {
        let mut mr = MessageReader::new();
        let mut b = "ok".as_bytes();
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Ok("".to_string()));

        let mut mr = MessageReader::new();
        let mut b = "ok foo".as_bytes();
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Ok("foo".to_string()));

        let mut mr = MessageReader::new();
        let mut b = "ok foo\r\nok bar\r\nquit".as_bytes();
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Ok("foo".to_string()));
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Ok("bar".to_string()));
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Quit);

        let mut mr = MessageReader::new();
        let mut b = "ok foo\nok bar\nquit\n".as_bytes();
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Ok("foo".to_string()));
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Ok("bar".to_string()));
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Quit);

        let mut mr = MessageReader::new();
        let mut b = "\r\n\r\nok foo".as_bytes();
        let ret = mr.read_msg("epz", &mut b).unwrap_err();
        assert_eq!(ret, ReadMsgError::TryAgain);
        let ret = mr.read_msg("epz", &mut b).unwrap_err();
        assert_eq!(ret, ReadMsgError::TryAgain);
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Ok("foo".to_string()));

        let mut mr = MessageReader::new();
        let mut b = "direct \"foo bar\" bar xx \t  foo".as_bytes();
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Direct(vec![
            "foo bar".to_string(),
            "bar".to_string(),
            "xx".to_string(),
            "foo".to_string(),
        ]));

        let mut mr = MessageReader::new();
        let mut b = "error \"foo bar\" bar xx \t  foo".as_bytes();
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Error(vec![
            "foo bar".to_string(),
            "bar".to_string(),
            "xx".to_string(),
            "foo".to_string(),
        ]));

        let mut mr = MessageReader::new();
        let mut b = "json 4\n[\"\"]\n".as_bytes();
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Payload("json", "[\"\"]".to_string().as_bytes().to_vec()));
    }
}

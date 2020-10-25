use wlambda::VVal;
use std::sync::*;
use std::sync::atomic::AtomicBool;
use std::net::TcpStream;
use std::net::TcpListener;
use std::fmt::Write;

use crate::sync_event::SyncEvent;

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

fn write_msg_arg(w: &mut std::fmt::Write, arg: &str) {
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

fn write_msg_args(w: &mut std::fmt::Write, args: &[String]) {
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
            Msg::Ok(arg)             => None,
            Msg::Hello(args)         => Some(Msg::Ok("hello".to_string())),
            Msg::Error(args)         => Some(Msg::Ok("error".to_string())),
            Msg::Direct(args)        => Some(Msg::Ok("direct".to_string())),
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

        use std::io::BufRead;
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


#[derive(Debug, Clone)]
pub enum Event {
    RecvMessage(Msg),
    SentMessage,
    ConnectionAvailable,
    ReaderConnectionAvailable,
    ConnectError(String),
    LogErr(String),
    LogInf(String),
}

#[derive(Debug, Clone)]
pub struct EventCtx {
    pub user_id:    u64,
    pub event:      Event,
}

impl EventCtx {
    pub fn new(user_id: u64, event: Event) -> Self {
        Self { user_id, event }
    }

    pub fn new_queue() -> (mpsc::Sender<EventCtx>, mpsc::Receiver<EventCtx>) {
        std::sync::mpsc::channel()
    }
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
                    Some(std::time::Duration::from_millis(op_timeout_ms)))
                    .expect("Setting read timeout");
                stream.set_write_timeout(
                    Some(std::time::Duration::from_millis(op_timeout_ms)))
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
        Some(std::time::Duration::from_millis(op_timeout_ms)))
        .expect("Setting read timeout");
    stream.set_write_timeout(
        Some(std::time::Duration::from_millis(op_timeout_ms)))
        .expect("Setting write timeout");
}

/// A persistent TCP connection
///
/// - Connection communicates events via mpsc
/// - Connection can be controlled via Connection-Handle
/// - A connect thread cares about reconnecting the connection once lost.
/// - The Read/Write threads care about reading and writing data
/// - Everything can be shut down using the stop-Boolean
///   - Thread handles are kept around for easy cleanup

pub struct TCPCSVConnection {
    /// Controls the maximum amount a thread blocks in an I/O operation
    /// in the background. This is for making sure, the connection can be
    /// shut down in the given time.
    op_timeout_ms:      u64,
    reader:             Option<std::thread::JoinHandle<()>>,
    writer:             Option<std::thread::JoinHandle<()>>,
    connector:          Option<std::thread::JoinHandle<()>>,
    writer_tx:          mpsc::Sender<Option<Msg>>,
    writer_rx:          Option<mpsc::Receiver<Option<Msg>>>,
    event_tx:           mpsc::Sender<EventCtx>,
    new_writer_stream:  SyncEvent<TcpStream>,
    new_reader_stream:  SyncEvent<TcpStream>,
    stop:               std::sync::Arc<AtomicBool>,
    need_reconnect:     std::sync::Arc<AtomicBool>,
    user_id:            u64,
}

macro_rules! send_event {
    ($where: expr, $user_id: expr, $event_tx: expr, $event: expr) => {
        if let Err(e) = $event_tx.send(EventCtx::new($user_id, $event)) {
            let _ =
                $event_tx.send(
                    EventCtx::new($user_id,
                        Event::LogErr(
                            format!("error sending event from {}: {}", $where, e))));
            break;
        }
    }
}

impl TCPCSVConnection {
    pub fn new(user_id: u64, e_tx: mpsc::Sender<EventCtx>) -> Self {
        let (w_tx, w_rx) = std::sync::mpsc::channel();

        let mut con = Self {
            user_id,
            op_timeout_ms:      1000,
            reader:             None,
            writer:             None,
            connector:          None,
            writer_tx:          w_tx,
            writer_rx:          Some(w_rx),
            event_tx:           e_tx,
            // Writer thread needs to clear the writer_rx queue upon receiving
            // the new stream and needs to send the "connected" event through the
            // event_tx
            new_writer_stream:  SyncEvent::new(),
            new_reader_stream:  SyncEvent::new(),
            stop:               std::sync::Arc::new(AtomicBool::new(false)),
            need_reconnect:     std::sync::Arc::new(AtomicBool::new(false)),
        };

        con.reader    = Some(con.start_reader());
        con.writer    = Some(con.start_writer());

        con
    }

    pub fn send(&mut self, msg: Msg) -> Result<(), std::sync::mpsc::SendError<Option<Msg>>> {
        self.writer_tx.send(Some(msg))?;
        Ok(())
    }

    fn start_reader(&mut self) -> std::thread::JoinHandle<()> {
        let stop                  = self.stop.clone();
        let need_reconnect        = self.need_reconnect.clone();
        let new_reader_stream     = self.new_reader_stream.clone();
        let event_tx              = self.event_tx.clone();
        let user_id               = self.user_id;

        std::thread::spawn(move || {
            let mut stream  = None;
            let mut ep      = String::from("?:?");
            let mut reader  = None;

            println!("READER START");

            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                if stream.is_none() {
                    reader = None;
                }

                if let Some(s) = &stream {
                    println!("R: {}", ep);
                } else {
//                    println!("R----");
                }

                if stream.is_none() || new_reader_stream.is_available() {
                    let new_stream =
                        new_reader_stream.recv_timeout(
                            std::time::Duration::from_millis(250));

                    if let Some(new_stream) = new_stream {
                        ep =
                            format!("{}",
                                new_stream.local_addr().unwrap());
                        stream = Some(std::io::BufReader::new(new_stream));
                        reader = Some(MessageReader::new());

                    } else {
                        continue;
                    }
                }

                if let Some(rd) = &mut reader {
                    let mut try_again = true;
                    while try_again {
                        try_again = false;

                        match rd.read_msg(&ep, stream.as_mut().unwrap()) {
                            Ok(msg) => {
                                send_event!("reader", user_id, event_tx,
                                    Event::RecvMessage(msg));
                            },
                            Err(ReadMsgError::Timeout) => {
                                //d// println!("TIMEOUT");
                                // nop
                            },
                            Err(ReadMsgError::TryAgain) => {
                                //d// println!("TRYAGAIN");
                                try_again = true;
                            },
                            Err(e) => {
                                send_event!("reader", user_id, event_tx,
                                    Event::LogErr(
                                        format!("read error: {:?}", e)));

                                stream.take().unwrap().into_inner()
                                      .shutdown(std::net::Shutdown::Both);
                                stream = None;
                                need_reconnect.store(
                                    true, std::sync::atomic::Ordering::Relaxed);
                            },
                        }
                    }
                }
            }

            stream = None;
            reader = None;
        })
    }

    fn start_writer(&mut self) -> std::thread::JoinHandle<()> {
        let writer =
            self.writer_rx.take()
                .expect("start_writer to be called only once!");
        let stop                   = self.stop.clone();
        let need_reconnect         = self.need_reconnect.clone();
        let new_writer_stream      = self.new_writer_stream.clone();
        let event_tx               = self.event_tx.clone();
        let user_id                = self.user_id;

        std::thread::spawn(move || {
            let mut stream        : Option<TcpStream> = None;
            let mut cur_frame     : Option<Vec<u8>>   = None;
            let mut cur_write_ptr : Option<&[u8]>     = None;
            println!("WRITER START");

            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some(s) = &stream {
                    println!("W: {}", s.local_addr().unwrap());
                }

                if stream.is_none() || new_writer_stream.is_available() {
                    stream =
                        new_writer_stream.recv_timeout(
                            std::time::Duration::from_millis(250));

                    if stream.is_some() {
                        cur_frame     = None;
                        cur_write_ptr = None;

                        while let Ok(_) = writer.try_recv() {
                            // nop
                        }

                        send_event!("writer", user_id, event_tx, Event::ConnectionAvailable);
                    } else {
                        continue;
                    }
                }

                if cur_frame.is_none() {
                    match writer.recv_timeout(std::time::Duration::from_millis(1000)) {
                        Ok(None) => { break; },
                        Ok(Some(msg)) => {
                            cur_frame     = Some(msg.to_frame());
                            cur_write_ptr = Some(&cur_frame.as_ref().unwrap()[..]);
                        },
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            continue;
                        },
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            break;
                        },
                    }

                } else if let Some(strm) = stream.as_mut() {
                    use std::io::Write;

                    match strm.write(cur_write_ptr.unwrap()) {
                        Ok(n) => {
                            if n == 0 {
                                cur_write_ptr = None;
                                cur_frame     = None;
                                stream        = None;
                                continue;
                            }

                            cur_write_ptr =
                                Some(&cur_write_ptr.take().unwrap()[n..]);

                            if cur_write_ptr.as_ref().unwrap().len() == 0 {
                                cur_write_ptr = None;
                                cur_frame     = None;

                                send_event!("writer", user_id, event_tx, Event::SentMessage);
                            }
                        },
                        Err(e) => {
                            match e.kind() {
                                  std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::Interrupted
                                | std::io::ErrorKind::WouldBlock => { continue; },
                                _ => {
                                    cur_write_ptr = None;
                                    cur_frame     = None;
                                    stream.take().unwrap()
                                          .shutdown(std::net::Shutdown::Both);

                                    send_event!("writer", user_id, event_tx,
                                        Event::LogErr(
                                            format!("write error: {}", e)));
                                    need_reconnect.store(
                                        true, std::sync::atomic::Ordering::Relaxed);
                                    continue;
                                }
                            }
                        }
                    }
                }
            }

            stream        = None;
            cur_frame     = None;
            cur_write_ptr = None;
        })
    }

    pub fn connect(&mut self, ep: &str) {
        if self.connector.is_some() {
            return;
        }

        self.connector = Some(self.start_connect(ep.to_string()));

        self.need_reconnect.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn start_connect(&mut self, ep: String) -> std::thread::JoinHandle<()> {
        let stop           = self.stop.clone();
        let need_reconnect = self.need_reconnect.clone();
        let event_tx       = self.event_tx.clone();
        let op_timeout_ms  = self.op_timeout_ms;
        let user_id        = self.user_id;

        let new_writer_stream = self.new_writer_stream.clone();
        let new_reader_stream = self.new_reader_stream.clone();

        std::thread::spawn(move || {
            loop {
                while !stop.load(std::sync::atomic::Ordering::Relaxed)
                      && !need_reconnect.load(std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                }

                if stop.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }

                use std::str::FromStr;
                let mut stream =
                    TcpStream::connect_timeout(
                        &std::net::SocketAddr::from_str(&ep)
                         .expect("Valid endpoint address"),
                        std::time::Duration::from_millis(op_timeout_ms));

                match stream {
                    Ok(mut stream) => {
                        set_stream_settings(&mut stream, op_timeout_ms);
                        need_reconnect.store(false, std::sync::atomic::Ordering::Relaxed);

                        new_writer_stream.send(
                            stream.try_clone()
                                  .expect("cloing a TcpStream to work"));
                        new_reader_stream.send(stream);

                    },
                    Err(e) => {
                        send_event!("connector", user_id, event_tx,
                            Event::ConnectError(format!("{}", e)));

                        // TODO: Make exponential time back off up to 1? minute
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        continue;
                    },
                }

                //d// eprintln!("reader ends (2)");
            }
        })
    }

    pub fn shutdown(&mut self) {
//        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
//        if let Some(wtx) = &self.writer_tx {
//            if let Err(_) = wtx.send(None) {
//            }
//        }
//        std::mem::replace(&mut self.reader_rx, None);
//        std::mem::replace(&mut self.writer_tx, None);
//        let rd = std::mem::replace(&mut self.reader, None);
//        if let Some(rd) = rd {
//            if let Err(_) = rd.join() {
//            }
//        }
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
        assert_eq!(rest, "bar exop");

        let (arg, rest) = next_arg("\"fo\\\"o\" \"bar oo\" exop");
        assert_eq!(arg, "fo\"o");
        assert_eq!(rest, "\"bar oo\" exop");

        let (arg, rest) = next_arg(rest);
        assert_eq!(arg, "bar oo");
        assert_eq!(rest, "exop");

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
        let mut b = "json 5\n[\"\"]\njson 0\njson 6\n".as_bytes();
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Payload("json", None, "[\"\"]\n".to_string().as_bytes().to_vec()));

        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Payload("json", None, "".to_string().as_bytes().to_vec()));
        let ret = mr.read_msg("epz", &mut b);
        assert_eq!(ret, Err(ReadMsgError::EOF));

        let mut mr = MessageReader::new();
        let mut b = "payload 5 foo bar it'sJSON\n[\"\"]\n".as_bytes();
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Payload("payload", Some(vec![
            "foo".to_string(),
            "bar".to_string(),
            "it'sJSON".to_string(),
        ]), "[\"\"]\n".to_string().as_bytes().to_vec()));

        let mut mr = MessageReader::new();
        let mut b = "hello foo ba \"foo\"\n".as_bytes();
        let ret = mr.read_msg("epz", &mut b).unwrap();
        assert_eq!(ret, Msg::Hello(vec![
            "foo".to_string(),
            "ba".to_string(),
            "foo".to_string(),
        ]));
    }

    #[test]
    fn check_msg_to_frame() {
        assert_eq!(
            Msg::Ok("foo".to_string()).to_frame(),
            b"ok foo\r\n");
        assert_eq!(Msg::Quit.to_frame(), b"quit\r\n");
        assert_eq!(Msg::Ping.to_frame(), b"ping\r\n");
        assert_eq!(
            Msg::Hello(vec![
                "foo".to_string(),
                "bar xxx".to_string(),
                "bar \r\n xxx".to_string(),
                "\"".to_string(),
            ]).to_frame(),
            b"hello foo \"bar xxx\" \"bar \\r\\n xxx\" \"\\\"\"\r\n");
        assert_eq!(
            Msg::Direct(vec![
                "foo".to_string(),
                "bar xxx".to_string(),
                "bar \r\n xxx".to_string(),
                "\"".to_string(),
            ]).to_frame(),
            b"direct foo \"bar xxx\" \"bar \\r\\n xxx\" \"\\\"\"\r\n");
        assert_eq!(
            Msg::Error(vec![
                "foo".to_string(),
                "bar xxx".to_string(),
                "bar \r\n xxx".to_string(),
                "\"".to_string(),
            ]).to_frame(),
            b"error foo \"bar xxx\" \"bar \\r\\n xxx\" \"\\\"\"\r\n");
    }
}

use wlambda::VVal;
use std::sync::*;
use std::sync::atomic::AtomicBool;
use std::net::TcpStream;
use std::net::TcpListener;
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
                println!("READL: [{}]", line);

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
    RecvMessage(Msg),
    SentMessage,
    ConnectionAvailable,
    ConnectError(String),
    LogErr(String),
    LogInf(String),
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
    event_tx:           mpsc::Sender<Event>,
    pub event_rx:       mpsc::Receiver<Event>,
    next_writer_stream: std::sync::Arc<std::sync::Mutex<Option<TcpStream>>>,
    next_reader_stream: std::sync::Arc<std::sync::Mutex<Option<TcpStream>>>,
    stop:               std::sync::Arc<AtomicBool>,
    need_reconnect:     std::sync::Arc<AtomicBool>,
}

macro_rules! send_event {
    ($where: expr, $event_tx: expr, $event: expr) => {
        if let Err(e) = $event_tx.send($event) {
            let _ =
                $event_tx.send(
                    Event::LogErr(
                        format!("error sending event from {}: {}", $where, e)));
            break;
        }
    }
}


macro_rules! lock_mutex {
    ($where: expr, $event_tx: expr, $mutex: expr, $ok_var: ident, $do: block) => {
        match $mutex.lock() {
            Ok(mut $ok_var) => $do,
            Err(e) => {
                send_event!($where, $event_tx,
                    Event::LogErr(
                        format!("error getting lock: {}", e)));
                break;
            },
        }
    }
}

impl TCPCSVConnection {
    pub fn new() -> Self {
        let (w_tx, w_rx) = std::sync::mpsc::channel();
        let (e_tx, e_rx) = std::sync::mpsc::channel();

        Self {
            op_timeout_ms:      10000,
            reader:             None,
            writer:             None,
            connector:          None,
            writer_tx:          w_tx,
            writer_rx:          Some(w_rx),
            event_tx:           e_tx,
            event_rx:           e_rx,
            // Writer thread needs to clear the writer_rx queue upon receiving
            // the new stream and needs to send the "connected" event through the
            // event_tx
            next_writer_stream: std::sync::Arc::new(std::sync::Mutex::new(None)),
            next_reader_stream: std::sync::Arc::new(std::sync::Mutex::new(None)),
            stop:               std::sync::Arc::new(AtomicBool::new(false)),
            need_reconnect:     std::sync::Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn send(&mut self, msg: Msg) -> Result<(), std::sync::mpsc::SendError<Option<Msg>>> {
        self.writer_tx.send(Some(msg))?;
        Ok(())
    }

    fn start_reader(&mut self) -> std::thread::JoinHandle<()> {
        let stop               = self.stop.clone();
        let need_reconnect     = self.need_reconnect.clone();
        let next_reader_stream = self.next_reader_stream.clone();
        let event_tx           = self.event_tx.clone();

        std::thread::spawn(move || {
            let mut stream  = None;
            let mut ep      = String::from("?:?");
            let mut reader  = None;

            while stop.load(std::sync::atomic::Ordering::Relaxed) {
                lock_mutex!("reader", event_tx, next_reader_stream, stream_opt, {
                    if let Some(new_stream) = (*stream_opt).take() {
                        stream = Some(std::io::BufReader::new(new_stream));
                        reader = Some(MessageReader::new());

                    } else if stream.is_none() {
                        // Sleep a bit, as long as we haven't got a stream at all:
                        // FIXME: Use a condition variable!
                        std::thread::sleep(
                            std::time::Duration::from_millis(250));
                    }
                });

                if let Some(rd) = &mut reader {
                    let mut try_again = true;
                    while try_again {
                        try_again = false;

                        match rd.read_msg(&ep, stream.as_mut().unwrap()) {
                            Ok(msg) => {
                            },
                            Err(ReadMsgError::Timeout) => {
                                // nop
                            },
                            Err(ReadMsgError::TryAgain) => {
                                try_again = true;
                            },
                            Err(e) => {
                                send_event!("reader", event_tx,
                                    Event::LogErr(
                                        format!("read error: {:?}", e)));
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
        let stop               = self.stop.clone();
        let need_reconnect     = self.need_reconnect.clone();
        let next_writer_stream = self.next_writer_stream.clone();
        let event_tx           = self.event_tx.clone();

        std::thread::spawn(move || {
            let mut stream                          = None;
            let mut cur_frame     : Option<Vec<u8>> = None;
            let mut cur_write_ptr : Option<&[u8]>   = None;

            while stop.load(std::sync::atomic::Ordering::Relaxed) {
                lock_mutex!("writer", event_tx, next_writer_stream, stream_opt, {
                    if let Some(new_stream) = (*stream_opt).take() {
                        stream = Some(new_stream);

                        cur_frame     = None;
                        cur_write_ptr = None;

                        while let Ok(_) = writer.try_recv() {
                            // nop
                        }

                        send_event!("writer", event_tx, Event::ConnectionAvailable);

                    } else if stream.is_none() {
                        // Sleep a bit, as long as we haven't got a stream at all:
                        // FIXME: Use a condition variable!
                        std::thread::sleep(
                            std::time::Duration::from_millis(250));
                    }
                });

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

                                send_event!("writer", event_tx, Event::SentMessage);
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
                                    stream        = None;

                                    send_event!("writer", event_tx,
                                        Event::LogErr(
                                            format!("write error: {}", e)));
                                    continue;
                                }
                            }
                        }
                    }
                }

                // if got stream:
                //   - look for messages to be written
                //   - write them, send a "written" event
                //   - once something has been written and the write buffer is empty,
                //     send a "write_queue_empty" event
            }

            stream        = None;
            cur_frame     = None;
            cur_write_ptr = None;
        })
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

    pub fn start_connect(&mut self, ep: String) -> std::thread::JoinHandle<()> {
        let stop           = self.stop.clone();
        let need_reconnect = self.need_reconnect.clone();
        let event_tx       = self.event_tx.clone();
        let op_timeout_ms  = self.op_timeout_ms;

        let next_writer_stream = self.next_writer_stream.clone();
        let next_reader_stream = self.next_reader_stream.clone();

        std::thread::spawn(move || {
            loop {
                while !stop.load(std::sync::atomic::Ordering::Relaxed)
                      && !need_reconnect.load(std::sync::atomic::Ordering::Relaxed) {
                    // FIXME: Use a condition variable!!!111
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

                        lock_mutex!("connector", event_tx, next_writer_stream, stream_opt, {
                            *stream_opt =
                                Some(stream.try_clone().expect("cloing a TcpStream to work"));
                        });
                        lock_mutex!("connector", event_tx, next_reader_stream, stream_opt, {
                            *stream_opt = Some(stream);
                        });

                    },
                    Err(e) => {
                        send_event!("connector", event_tx,
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

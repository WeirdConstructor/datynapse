use std::sync::*;
use std::sync::atomic::AtomicBool;
use std::net::TcpStream;
use std::net::TcpListener;

use crate::sync_event::SyncEvent;
use crate::msg::*;

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

#[derive(Debug, Clone)]
pub enum Event {
    RecvMessage(Msg),
    SentMessage,
    ConnectionAvailable,
    ConnectionLost,
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

enum ServerInternalEvent {
    NewConnection(TcpStream),
    RemoveConnection(u64),
    SendMessage(u64, Msg),
}

pub struct TCPCSVServer {
    event_tx:       mpsc::Sender<EventCtx>,
    writer_tx:      mpsc::Sender<ServerInternalEvent>,
    writer_rx:      Option<mpsc::Receiver<ServerInternalEvent>>,
    init_id:        u64,
    op_timeout_ms:  u64,
}

impl TCPCSVServer {
    pub fn new(init_id: u64, event_tx: mpsc::Sender<EventCtx>) -> Self {
        let (writer_tx, writer_rx) = mpsc::channel();
        Self {
            writer_rx:      Some(writer_rx),
            op_timeout_ms:  1000,
            init_id,
            writer_tx,
            event_tx,
        }
    }

    pub fn send(&mut self, id: u64, msg: Msg) {
        self.writer_tx.send(ServerInternalEvent::SendMessage(id, msg)).is_ok();
    }

    pub fn check_event_to_handle(&mut self, ev: &EventCtx) {
        match ev.event {
            Event::ConnectionLost => {
                self.writer_tx.send(
                    ServerInternalEvent::RemoveConnection(ev.user_id)).is_ok();
            },
            _ => (),
        }
    }

    pub fn start_listener(&mut self, ep: &str) -> Result<(), std::io::Error> {
        let l = TcpListener::bind(ep)?;

        let op_timeout_ms = self.op_timeout_ms;
        let event_tx      = self.event_tx.clone();
        let writer_tx     = self.writer_tx.clone();

        std::thread::spawn(move || {
            for stream in l.incoming() {
                let mut stream =
                    match stream {
                        Ok(s) => s,
                        Err(e) => {
                            send_event!("listener", 0, event_tx,
                                Event::ConnectError(format!("{}", e)));
                            break;
                        }
                    };

                set_stream_settings(&mut stream, op_timeout_ms);

                writer_tx.send(ServerInternalEvent::NewConnection(stream)).is_ok();
            }
        });

        let writer_rx = self.writer_rx.take().unwrap();
        let event_tx = self.event_tx.clone();
        let mut id_counter = self.init_id;

        std::thread::spawn(move || {
            let mut connections = std::collections::HashMap::new();
            loop {
                match writer_rx.recv() {
                    Ok(ServerInternalEvent::NewConnection(stream)) => {
                        id_counter += 1;
                        let mut con =
                            TCPCSVConnection::new(id_counter, event_tx.clone());
                        con.set_stream(stream);
                        connections.insert(id_counter, Some(con));

                        println!("NEW CON");
                    },
                    Ok(ServerInternalEvent::RemoveConnection(id)) => {
                        if let Some(con) = connections.remove(&id) {
                            con.unwrap().shutdown();
                        }
                    },
                    Ok(ServerInternalEvent::SendMessage(id, msg)) => {
                        if let Some(con) = connections.get_mut(&id) {
                            con.as_mut().unwrap().send(msg);
                        }
                    },
                    Err(e) => {
                        send_event!("listener_mgmt", 0, event_tx,
                            Event::LogErr(
                                format!("Error reading event: {}", e)));
                        break;
                    },
                }
            }
        });

        Ok(())
    }
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

                if let Some(_s) = &stream {
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

                                send_event!("reader", user_id, event_tx,
                                    Event::ConnectionLost);
                            },
                        }
                    }
                }
            }
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

                                    send_event!("reader", user_id, event_tx,
                                        Event::ConnectionLost);
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    pub fn connect(&mut self, ep: &str) {
        if self.connector.is_some() {
            return;
        }

        self.connector = Some(self.start_connect(ep.to_string()));

        self.need_reconnect.store(true, std::sync::atomic::Ordering::Relaxed);
    }


    pub fn set_stream(&mut self, stream: TcpStream) {
        self.need_reconnect.store(false, std::sync::atomic::Ordering::Relaxed);
        self.new_writer_stream.send(
            stream.try_clone()
                  .expect("cloing a TcpStream to work"));
        self.new_reader_stream.send(stream);
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
                let stream =
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
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(con) = self.connector.take() {
            con.join();
        }
        if let Some(wr) = self.writer.take() {
            wr.join();
        }
        if let Some(rd) = self.reader.take() {
            rd.join();
        }
        println!("*****=========> SHUTDOWN!");

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

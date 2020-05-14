use wlambda::VVal;
use std::sync::*;
use std::sync::atomic::AtomicBool;
use std::net::TcpStream;
use std::net::TcpListener;

#[derive(Debug, Clone)]
pub enum Event {
    IncomingMessage(String),
    DroppedMessage,
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

pub struct TCPCSVConnection {
    op_timeout_ms:      u64,
    reader:             Option<std::thread::JoinHandle<()>>,
    writer_tx:          Option<mpsc::Sender<Option<Vec<u8>>>>,
    pub reader_rx:      Option<mpsc::Receiver<Event>>,
    stop:               std::sync::Arc<AtomicBool>,
}

fn spawn_writer_thread(mut writer_stream: TcpStream,
                       wr_ep: String,
                       writer_event_tx: mpsc::Sender<Event>,
                       writer_rx: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<Option<Vec<u8>>>>>)
    -> std::thread::JoinHandle<()>
{
    std::thread::spawn(move || {
        use std::io::Write;
        match writer_rx.lock() {
            Ok(wr_rx) => {
                while let Ok(Some(bytes)) = wr_rx.recv() {
                    if let Err(e) = writer_stream.write_all(&bytes) {
                        match e.kind() {
                            std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::WouldBlock => {
                                if let Err(e) = writer_event_tx.send(
                                    Event::DroppedMessage) {
                                }
                                continue;
                            },
                            _ => {
                                if let Err(e) = writer_event_tx.send(
                                    Event::DroppedMessage) {
                                }

                                if let Err(e) = writer_event_tx.send(
                                    Event::ConnectError(
                                        format!("Write error from {}: {}",
                                                wr_ep, e))) {
                                }
                                break;
                            }
                        }
                    }

                    if let Err(e) = writer_stream.flush() {
                        if let Err(e) = writer_event_tx.send(
                            Event::DroppedMessage) {
                        }

                        if let Err(_) = writer_event_tx.send(
                            Event::ConnectError(
                                format!("Write flush error from {}: {}",
                                        wr_ep, e))) {
                        };
                        break;
                    }
                    if let Err(e) = writer_event_tx.send(
                        Event::FlushedMessage) {
                    }
                }
            },
            Err(e) => {
                if let Err(_) = writer_event_tx.send(
                    Event::ConnectError(
                        format!("Write queue mutex error from {}: {}",
                                wr_ep, e))) {
                };
            },
        }

        //d// eprintln!("writer ends");
    })
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

    pub fn connect(&mut self) {
        let (wr_tx, wr_rx)    = std::sync::mpsc::channel();
        let (event_tx, rd_rx) = std::sync::mpsc::channel();

        let wr_stop_tx = wr_tx.clone();

        self.writer_tx = Some(wr_tx);
        self.reader_rx = Some(rd_rx);

        self.stop = std::sync::Arc::new(AtomicBool::new(false));

        let wr_rx = std::sync::Arc::new(std::sync::Mutex::new(wr_rx));

        let op_timeout_ms = self.op_timeout_ms;
        let ep = String::from("127.0.0.1:57322");
        let stop_reader = self.stop.clone();
        let reader = std::thread::spawn(move || {
            loop {
                // clear write buffer
                if let Ok(wrx) = wr_rx.lock() {
                    for _data in wrx.try_iter() {
                        if let Err(e) = event_tx.send(
                            Event::DroppedMessage) {
                        }
                    }
                }

                use std::str::FromStr;
                let mut stream =
                    TcpStream::connect_timeout(
                        &std::net::SocketAddr::from_str(&ep)
                         .expect("Valid endpoint address"),
                        std::time::Duration::from_millis(op_timeout_ms));

                match stream {
                    Ok(mut stream) => {
                        stream.set_nodelay(true)
                            .expect("Setting TCP no delay");
                        stream.set_read_timeout(
                            Some(std::time::Duration::from_millis(op_timeout_ms / 10)))
                            .expect("Setting read timeout");
                        stream.set_write_timeout(
                            Some(std::time::Duration::from_millis(op_timeout_ms / 10)))
                            .expect("Setting write timeout");

                        let local_addr_str = match stream.local_addr() {
                            Ok(s)  => format!("{}", s),
                            Err(e) => format!("no-local-addr({})", e),
                        };

                        let wr_ep           = ep.clone();
                        let writer_event_tx = event_tx.clone();
                        let writer_rx       = wr_rx.clone();
                        let mut writer_stream =
                            stream
                            .try_clone()
                            .expect("Cloning stream should work for writer!");
                        let writer =
                            spawn_writer_thread(
                                writer_stream, wr_ep, writer_event_tx, writer_rx);

                        if let Err(e) =
                            event_tx.send(
                                Event::Connected(
                                    format!("Connected to {} from {}",
                                            ep, local_addr_str))) {

                            wr_stop_tx.send(None);
                            writer.join();
                            return;
                        };

                        use std::io::BufRead;
                        let mut buf = std::io::BufReader::new(stream);
                        loop {
                            if stop_reader.load(std::sync::atomic::Ordering::Relaxed) {
                                break;
                            }

                            let mut line = String::new();
                            match buf.read_line(&mut line) {
                                Ok(n) => {
                                    if n == 0 {
                                        if let Err(_) = event_tx.send(
                                            Event::ConnectError(
                                                format!("EOF from {}", ep))) {
                                        };
                                        break;
                                    }

                                    if let Err(e) =
                                        event_tx.send(Event::IncomingMessage(line)) {
                                        break;
                                    };
                                },
                                Err(e) => {
                                    match e.kind() {
                                        std::io::ErrorKind::TimedOut
                                        | std::io::ErrorKind::WouldBlock => {
                                            continue;
                                        },
                                        _ => {
                                            if let Err(_) = event_tx.send(
                                                Event::ConnectError(
                                                    format!("Read error from {}: {}",
                                                            ep, e))) {
                                            };
                                            break;
                                        }
                                    }
                                },
                            }
                        }

                        wr_stop_tx.send(None);
                        writer.join();
                    },
                    Err(err) => {
                        if let Err(e) =
                            event_tx.send(
                                Event::ConnectError(
                                    format!("Couldn't connect to {}: {:?}", ep, err))) {
                            break;
                        }
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

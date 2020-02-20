use wlambda::VVal;
use std::sync::*;
use std::net::TcpStream;


#[derive(Debug, Clone)]
pub enum Event {
    IncomingMessage(String),
    ConnectError(String),
    Connected(String),
    Quit,
}

pub struct TCPCSVConnection {
    op_timeout_ms:      u64,
    reader:             Option<std::thread::JoinHandle<()>>,
    writer_tx:          Option<mpsc::Sender<Vec<u8>>>,
    pub reader_rx:          Option<mpsc::Receiver<Event>>,
}

impl TCPCSVConnection {
    pub fn new() -> Self {
        Self {
            op_timeout_ms: 10000,
            reader:        None,
            writer_tx:     None,
            reader_rx:     None,
        }
    }

    pub fn connect(&mut self) {
        let (wr_tx, wr_rx) = std::sync::mpsc::channel();
        let (event_tx, rd_rx) = std::sync::mpsc::channel();

        self.writer_tx = Some(wr_tx);
        self.reader_rx = Some(rd_rx);

        let op_timeout_ms = self.op_timeout_ms;
        let ep = String::from("127.0.0.1:57321");
        let reader = std::thread::spawn(move || {
            loop {
                use std::str::FromStr;
                let mut stream =
                    TcpStream::connect_timeout(
                        &std::net::SocketAddr::from_str(&ep)
                         .expect("Valid endpoint address"),
                        std::time::Duration::from_millis(op_timeout_ms));

                match stream {
                    Ok(stream) => {
                        stream.set_nodelay(true);
                        stream.set_read_timeout(
                            Some(std::time::Duration::from_millis(op_timeout_ms)));
                        stream.set_write_timeout(
                            Some(std::time::Duration::from_millis(op_timeout_ms)));

                        let local_addr_str = match stream.local_addr() {
                            Ok(s)  => format!("{}", s),
                            Err(e) => format!("no-local-addr({})", e),
                        };

                        event_tx.send(
                            Event::Connected(
                                format!("Connected to {} from {}",
                                        ep, stream.local_addr().unwrap())));

                        use std::io::BufRead;
                        let mut buf = std::io::BufReader::new(stream);
                        loop {
                            let mut line = String::new();
                            match buf.read_line(&mut line) {
                                Ok(n) => {
                                    if n == 0 {
                                        event_tx.send(
                                            Event::ConnectError(
                                                format!("EOF from {}", ep)));
                                        break;
                                    }

                                    event_tx.send(Event::IncomingMessage(line));
                                },
                                Err(e) => {
                                    event_tx.send(
                                        Event::ConnectError(
                                            format!("Read error from {}: {}",
                                                    ep, e)));
                                    break;
                                },
                            }
                        }
                    },
                    Err(err) => {
                        event_tx.send(
                            Event::ConnectError(
                                format!("Couldn't connect to {}: {:?}", ep, err)));
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        continue;
                    },
                }

//                while let Ok(bytes) = stdin_rx.recv() {
//                    if let Ok(s) = bw.write(&bytes) {
//                        if s == 0 { break; }
//                        if bw.flush().is_err() { break; }
//                    } else {
//                        break;
//                    }
//                };
//
//                let writer = std::thread::spawn(move || {
//                    while let Ok(bytes) = stdin_rx.recv() {
//                        if let Ok(s) = bw.write(&bytes) {
//                            if s == 0 { break; }
//                            if bw.flush().is_err() { break; }
//                        } else {
//                            break;
//                        }
//                    };
//                });
            }
        });
        std::mem::replace(&mut self.reader, Some(reader));
    }
}

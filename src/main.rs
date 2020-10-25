//mod detached_command;
//use detached_command::*;
mod tcp_csv_msg_connection;
mod sync_event;

use wlambda::VVal;

/*
There are different kinds of processes:

- Two-Way adapters
    - accept messages and generate a response immediately
    - responses sometimes 
- Call adapters
    - on incoming message a process is spawned and the output
      is sent as a new message
*/

#[derive(Debug, Copy, Clone)]
struct Label(usize);

#[derive(Debug, Clone)]
struct LabelInternalizer {
    labels:           std::collections::HashMap<String, (usize, u128, bool)>,
    resolve:          std::collections::HashMap<usize, String>,
    free_ids:         std::vec::Vec<usize>,
    id_counter:       usize,
    max_label_age_ms: u128,
}

impl LabelInternalizer {
    pub fn new() -> Self {
        Self {
            labels:           std::collections::HashMap::new(),
            resolve:          std::collections::HashMap::new(),
            free_ids:         vec![],
            id_counter:       0,
            max_label_age_ms: 6000, // 6kms = 1 minute
        }
    }

    fn next_id(&mut self) -> usize {
        if self.free_ids.is_empty() {
            self.id_counter += 1;
            self.id_counter
        } else {
            self.free_ids.pop().unwrap()
        }
    }

    fn get_timestamp(&self) -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Reading current time since epoch")
        .as_millis()
    }

    pub fn garbage_collect(&mut self) {
        let now = self.get_timestamp();

        let mut delete_labels = vec![];
        for (k, v) in self.labels.iter() {
            if (now - v.1) > self.max_label_age_ms {
                delete_labels.push(k.to_string());
            }
        }

        for lbl in delete_labels {
            let entry = self.labels.remove(&lbl).unwrap();
            self.resolve.remove(&entry.0);
            self.free_ids.push(entry.0);
        }
    }

    pub fn perm_lbl(&mut self, lbl: &str) -> Label {
        Label(self.get_label_id(lbl, true))
    }

    pub fn resolve(&self, lbl: Label) -> &str {
        match self.resolve.get(&lbl.0) {
            Some(lbl) => lbl,
            None => "",
        }
    }

    pub fn tmp_lbl(&mut self, lbl: &str) -> Label {
        Label(self.get_label_id(lbl, false))
    }

    fn get_label_id(&mut self, lbl: &str, permanent: bool) -> usize {
        let now = self.get_timestamp();
        match self.labels.get_mut(lbl) {
            Some(ref mut v) => {
                v.2 = permanent;
                v.1 = now;
                v.0
            },
            None => {
                let next_id = self.next_id();
                self.labels.insert(lbl.to_string(), (next_id, now, permanent));
                self.resolve.insert(next_id, lbl.to_string());
                next_id
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Message {
    dest:       Label,
    src:        Label,
    payload:    VVal,
}

#[derive(Debug, Copy, Clone)]
enum AdapterError {
    Again,
    End,
    Crash,
}

trait Adapter {
    fn try_poll_one(&mut self) -> Result<Message, AdapterError>;
    fn send(&mut self, msg: &Message);
    fn main_label(&self) -> Option<Label>;
}

#[derive(Debug, Clone)]
struct ConsoleAdapter {
    lbl: std::rc::Rc<std::cell::RefCell<LabelInternalizer>>,
    name: String,
    cnt: usize,
}

impl Adapter for ConsoleAdapter {
    fn main_label(&self) -> Option<Label> {
        Some(self.lbl.borrow_mut().perm_lbl(&self.name))
    }

    fn try_poll_one(&mut self) -> Result<Message, AdapterError> {
        if self.cnt > 0 {
            self.cnt -= 1;
            let v = VVal::vec();
            v.push(VVal::new_str("OK"));
            v.push(VVal::Int((self.cnt + 1) as i64));
            let src = self.main_label().unwrap();
            let mut fl = self.lbl.borrow_mut();
            return Ok(Message {
                src,
                dest:    fl.tmp_lbl("A"),
                payload: v,
            });
        } else {
            return Err(AdapterError::End);
        }
    }

    fn send(&mut self, msg: &Message) {
        println!("TO[{}/{}] FROM[{}]: {}",
            self.lbl.borrow().resolve(msg.dest),
            self.name,
            self.lbl.borrow().resolve(msg.src),
            msg.payload.s());
    }
}

struct Router {
    labels:      std::rc::Rc<std::cell::RefCell<LabelInternalizer>>,
    adapters:    std::vec::Vec<Box<dyn Adapter>>,
    routes:      std::vec::Vec<usize>, // maps labels to adapter index
    send_queue:  std::vec::Vec<Message>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            labels:
                std::rc::Rc::new(
                    std::cell::RefCell::new(
                        LabelInternalizer::new())),
            adapters:   vec![],
            routes:     vec![],
            send_queue: vec![],
        }
    }

    pub fn add_adapter(&mut self, adapter: Box<dyn Adapter>) {
        let ad_idx = self.adapters.len();

        if let Some(ad_lbl) = adapter.main_label() {
            if ad_lbl.0 >= self.routes.len() {
                self.routes.resize(ad_lbl.0 * 2, 0);
            }
            self.routes[ad_lbl.0] = ad_idx + 1;
        }

        self.adapters.push(adapter);
    }

    pub fn get_adapter(&mut self, idx: usize) -> Option<&mut Box<dyn Adapter>> {
        if idx == 0 { return None; }
        if idx >= self.adapters.len() { return None; }
        Some(&mut self.adapters[idx - 1])
    }

    pub fn get_adapter_for_label(&mut self, lbl: Label) -> Option<&mut Box<dyn Adapter>> {
        if lbl.0 >= self.routes.len() {
            return None;
        }
        self.get_adapter(self.routes[lbl.0])
    }

    pub fn one_process_cycle(&mut self) {
        while !self.send_queue.is_empty() {
            let msg = self.send_queue.pop().unwrap();
            if let Some(ad) = self.get_adapter_for_label(msg.dest) {
                ad.send(&msg);
            } else {
                println!("Dropped Message: {:?}", msg);
            }
        }

        for a in self.adapters.iter_mut() {
//            if let Some(ref mut adap) = a {
                match a.try_poll_one() {
                    Ok(msg) => {
                        self.send_queue.push(msg);
                    },
                    Err(e) => {
                        match e {
                            AdapterError::End => (),
                            _ => {
                                println!("ERR {:?}", e);
                            }
                        }
                    },
                }
//            }
        }
    }
}

fn route() {
//    let mut ads : std::vec::Vec<Box<dyn Adapter>> = vec![];
//
//    let lbl =
//        std::rc::Rc::new(
//            std::cell::RefCell::new(
//                LabelInternalizer::new()));
//
//    ads.push(Box::new(ConsoleAdapter { name: "A".to_string(), cnt: 2, lbl: lbl.clone() }));
//    ads.push(Box::new(ConsoleAdapter { name: "B".to_string(), cnt: 3, lbl: lbl.clone() }));
//
//    loop {
//    }

    let mut router = Router::new();

    router.add_adapter(Box::new(ConsoleAdapter {
        name: "A".to_string(),
        cnt: 2,
        lbl: router.labels.clone(),
    }));

    router.add_adapter(Box::new(ConsoleAdapter {
        name: "B".to_string(),
        cnt: 2,
        lbl: router.labels.clone(),
    }));

    loop {
        router.one_process_cycle();
    }
}

fn main() {
    use tcp_csv_msg_connection::{Event, Msg};

    let mut con = tcp_csv_msg_connection::TCPCSVConnection::new(12);
    con.connect("127.0.0.1:18444");

    let my_name       = "test run";
    let my_version    = "0.1-alpha";
    let proto_version = "1";

    let mut logged_in  = false;
    let mut hello_recv = false;
    let mut hello_sent = false;

    // TODO: measure time since last ping sent, if above => send ping
    // TODO: measure time since last "ok ping", if above => reconnect

    loop {
        let evctx = con.event_rx.recv().expect("no error");
        let id = evctx.user_id;
        let ev = evctx.event;

        assert_eq!(id, 12);

        //d// println!("EVENT: {:?}", ev);
        match ev {
            Event::ConnectionAvailable => {
                logged_in = false;
                con.send(Msg::Hello(vec![
                    my_name.to_string(),
                    my_version.to_string(),
                    proto_version.to_string()
                ]));
            },
            Event::RecvMessage(msg) => {
                match &msg {
                    Msg::Hello(args) => {
                        println!("Handshake with: {:?}", args);
                        hello_recv = true;
                        if hello_recv && hello_sent {
                            logged_in = true;
                            println!("Logged in!");
                        }
                    },
                    Msg::Ok(cmd) => {
                        match &cmd[..] {
                            "hello" => {
                                hello_sent = true;
                                if hello_recv && hello_sent {
                                    logged_in = true;
                                    println!("Logged in!");
                                }
                            },
                            _ => (),
                        }
                    },
                    Msg::Payload(p_cmd, args, payload) => {
                        if logged_in {
                            println!("RECV. PAYLOAD [{}]: {:?}", p_cmd, payload);
                        }
                    },
                    Msg::Ping => {
                        // NOP, handled by ok.
                    },
                    Msg::Quit => {
                    },
                    Msg::Direct(args) => {
                        if logged_in {
                            println!("DIRECT: {:?}", args);
                        }
                    },
                    Msg::Error(_) => {
                    },
                }

                if let Some(resp) = msg.ok_response() {
                    con.send(resp);
                }
            },
            Event::SentMessage => {
            },
            Event::ReaderConnectionAvailable => {
                println!("reader there!");
            },
            Event::LogErr(err) => {
                println!("** error: {}", err);
            },
            Event::LogInf(err) => {
                println!("** info: {}", err);
            },
            Event::ConnectError(err) => {
                println!("Connect error: {}", err);
            },
        }
    }


    {
//        let mut c = tcp_csv_msg_connection::TCPCSVConnection::new();
//
//        c.connect();
//        loop {
//            match c.reader_rx.as_mut().unwrap().recv() {
//                Ok(tcp_csv_msg_connection::Event::IncomingMessage(msg)) => {
//                    if msg == "quit\n" {
//                        break;
//                    }
//                    println!("RECV: {}", msg);
//                },
//                Ok(tcp_csv_msg_connection::Event::Connected(m)) => {
//                    println!("CONNECTED {}", m);
//                    for i in 0..100000 {
//                        c.send("AAAAAAAAAAAAAAAAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
//                    }
//                },
//                Ok(it) => {
//                    println!("RECV: {:?}", it);
//                },
//                Err(e) => {
//                    println!("ERROR: {}", e);
//                }
//            }
//        }
//
//        c.shutdown();
    }


    route();
//    let mut dc = DetachedCommand::start("wlambda", &[]).expect("X");
//
//    dc.send_str("10 + 20\n");
//    loop {
//        match dc.poll() {
//            Ok(()) => {
//                if dc.stderr_available() {
//                    println!("SE: {}", dc.recv_stderr());
//                }
//                if dc.stdout_available() {
//                    println!("SO: {}", dc.recv_stdout());
//                }
//            },
//            Err(err) => {
//                println!("stdout: [{}]", dc.recv_stdout());
//                println!("stderr: [{}]", dc.recv_stderr());
//                println!("Error in poll: {:?}", err);
//                break;
//            },
//        }
//    }
//
//    dc.shutdown();
}

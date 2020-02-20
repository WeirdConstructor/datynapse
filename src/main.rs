//mod detached_command;
//use detached_command::*;

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
struct ForgettingInternalizer {
    labels:           std::collections::HashMap<String, (usize, u128, bool)>,
    resolve:          std::collections::HashMap<usize, String>,
    free_ids:         std::vec::Vec<usize>,
    id_counter:       usize,
    max_label_age_ms: u128,
}

impl ForgettingInternalizer {
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
}

#[derive(Debug, Clone)]
struct ConsoleAdapter {
    lbl: std::rc::Rc<std::cell::RefCell<ForgettingInternalizer>>,
    name: String,
    cnt: usize,
}

impl Adapter for ConsoleAdapter {
    fn try_poll_one(&mut self) -> Result<Message, AdapterError> {
        if self.cnt > 0 {
            self.cnt -= 1;
            let v = VVal::vec();
            v.push(VVal::new_str("OK"));
            v.push(VVal::Int((self.cnt + 1) as i64));
            let mut fl = self.lbl.borrow_mut();
            return Ok(Message {
                src:     fl.tmp_lbl(&self.name),
                dest:    fl.tmp_lbl("@Out"),
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

fn route() {
    let mut ads : std::vec::Vec<Box<dyn Adapter>> = vec![];

    let lbl =
        std::rc::Rc::new(
            std::cell::RefCell::new(
                ForgettingInternalizer::new()));

    ads.push(Box::new(ConsoleAdapter { name: "A".to_string(), cnt: 2, lbl: lbl.clone() }));
    ads.push(Box::new(ConsoleAdapter { name: "B".to_string(), cnt: 3, lbl: lbl.clone() }));

    let mut snd_queue : std::vec::Vec<Message> = vec![];
    loop {
        let mut i = 0;
        while !snd_queue.is_empty() {
            let len = ads.len();
            ads[i % len].send(&snd_queue.pop().unwrap());
            i += 1;
        }

        for a in ads.iter_mut() {
//            if let Some(ref mut adap) = a {
                match a.try_poll_one() {
                    Ok(msg) => {
                        snd_queue.push(msg);
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

fn main() {
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

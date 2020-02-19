mod detached_command;
use detached_command::*;

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

#[derive(Debug, Clone)]
struct Message {
    dest:       String,
    src:        String,
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
            return Ok(Message {
                src: self.name.to_string(),
                dest: "@Out".to_string(),
                payload: v,
            });
        } else {
            return Err(AdapterError::End);
        }

    }
    fn send(&mut self, msg: &Message) {
        println!("TO[{}/{}] FROM[{}]: {}", msg.dest, self.name, msg.src, msg.payload.s());
    }
}

fn route() {
    let mut ads : std::vec::Vec<Box<dyn Adapter>> = vec![];

    ads.push(Box::new(ConsoleAdapter { name: "A".to_string(), cnt: 2 }));
    ads.push(Box::new(ConsoleAdapter { name: "B".to_string(), cnt: 3 }));

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

//mod detached_command;
//use detached_command::*;

//mod tcp_csv_msg_connection;

//mod sync_event;

mod event;
mod timer;
mod msg;
mod process;

use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::*;

use wlambda::*;

use crate::timer::*;
use crate::event::*;

enum Port {
    Process(Sender<process::Cmd>, std::thread::JoinHandle<()>),
}

impl Port {
    pub fn send_kill(self) {
        match self {
            Port::Process(sender, thread) => {
                sender.send(process::Cmd::Kill).is_ok();
            },
        }
    }

    pub fn join(self) {
        match self {
            Port::Process(sender, thread) => {
                drop(sender);
                thread.join();
            },
        }
    }
}

fn main() {
    let (event_tx, event_rx) = channel();
    let (timer_tx, timer_rx) = channel();

    let _timer_thread = start_timer_thread(event_tx.clone(), timer_rx);
    let cur_id = Rc::new(RefCell::new(0_u64));

    let callbacks = Rc::new(RefCell::new(HashMap::new()));
    let ports : Rc<RefCell<HashMap<u64, Port>>> = Rc::new(RefCell::new(HashMap::new()));

    let global = GlobalEnv::new_default();
    global.borrow_mut().add_func(
        "dn:wsmp:listen",
        move |_env: &mut Env, _argc: usize| {
            // create mpsc
            // draw new id
            // store mpsc sender in global register
            // clone global queue sender
            // create new thread
            // in thread setup wsmp:listen
            Ok(VVal::None)
        }, Some(0), Some(0));

    let cb_callbacks = callbacks.clone();
    global.borrow_mut().add_func(
        "dn:on",
        move |env: &mut Env, _argc: usize| {
            // store callback for given id in global register
            let id = env.arg(0).i() as u64;
            cb_callbacks.borrow_mut().insert(id, env.arg(1));
            Ok(VVal::None)
        }, Some(2), Some(2));

    let t_tx = timer_tx.clone();
    let c_id = cur_id.clone();
    global.borrow_mut().add_func(
        "dn:timer:oneshot",
        move |env: &mut Env, _argc: usize| {
            let dur =
                match env.arg(0).to_duration() {
                    Ok(dur) => dur,
                    Err(v)  => { return Ok(v); },
                };

            *c_id.borrow_mut() += 1;
            let cur_id = *c_id.borrow();

            t_tx.send(Timer::Oneshot(cur_id, dur.as_millis() as u64))
                 .is_ok(); // TODO: Handle error and log it!

            Ok(VVal::Int(cur_id as i64))
        }, Some(1), Some(1));

    let t_tx = timer_tx.clone();
    let c_id = cur_id.clone();
    global.borrow_mut().add_func(
        "dn:timer:interval",
        move |env: &mut Env, _argc: usize| {
            let dur =
                match env.arg(0).to_duration() {
                    Ok(dur) => dur,
                    Err(v)  => { return Ok(v); },
                };

            *c_id.borrow_mut() += 1;
            let cur_id = *c_id.borrow();

            let dur = dur.as_millis() as u64;
            t_tx.send(Timer::Interval(cur_id, dur, dur))
                 .is_ok(); // TODO: Handle error and log it!

            Ok(VVal::Int(cur_id as i64))
        }, Some(1), Some(1));

    {
        let ports = ports.clone();
        global.borrow_mut().add_func(
            "dn:kill",
            move |env: &mut Env, _argc: usize| {
                let id = env.arg(0).i() as u64;
                if let Some(port) = ports.borrow_mut().remove(&id) {
                    port.send_kill();
                }
                Ok(VVal::None)
            }, Some(1), Some(1));
    }

    global.borrow_mut().add_func(
        "dn:send",
        move |_env: &mut Env, _argc: usize| {
            // store callback for given id in global register
            Ok(VVal::None)
        }, Some(0), Some(0));

    {
        let ports    = ports.clone();
        let cur_id   = cur_id.clone();
        let event_tx = event_tx.clone();

        global.borrow_mut().add_func(
            "dn:process:start",
            move |env: &mut Env, argc: usize| {
                let (idx_offs, proto) =
                    if argc == 3 {
                        (1,
                            env.arg(0).with_s_ref(|s|
                                match s {
                                    "wsmp" => process::CmdProtocol::WSMP,
                                    _      => process::CmdProtocol::LineBased,
                                }))
                    } else {
                        (0, process::CmdProtocol::LineBased)
                    };

                *cur_id.borrow_mut() += 1;
                let cur_id = *cur_id.borrow();

                let event_tx = event_tx.clone();
                env.arg(idx_offs).with_s_ref(|cmd| {
                    let args : Vec<String> =
                        env.arg(idx_offs + 1)
                           .iter()
                           .map(|v| v.0.s_raw()).collect();

                    let (tx, rx) = channel();

                    let thread =
                        process::start(event_tx, rx, cur_id, cmd, &args, proto);

                    ports.borrow_mut()
                         .insert(cur_id, Port::Process(tx, thread));
                });

                Ok(VVal::Int(cur_id as i64))
            }, Some(2), Some(3));
    }

    global.borrow_mut().add_func(
        "dn:send",
        move |_env: &mut Env, _argc: usize| {
            // store callback for given id in global register
            Ok(VVal::None)
        }, Some(0), Some(0));


    let mut ctx = EvalContext::new(global);

    let argv : Vec<String> = std::env::args().collect();

    let filepath =
        if argv.len() > 1 {
            argv[1].to_string()
        } else {
            "init.wl".to_string()
        };

    ctx.eval_file(&filepath)
       .expect("correct evaluation of initialization file");

    loop {
        if let Ok(ev) = event_rx.recv() {
            let mut remove = None;

            match ev {
                Event::Timeout(id) => {
                    if let Some(cb) = callbacks.borrow().get(&id) {
                        let ret =
                            ctx.call(cb, &[VVal::Int(id as i64)]).expect("no error in cb");

                        if ret.with_s_ref(|s| s == "delete") {
                            remove = Some(id);
                        }
                    }
                },
                Event::Message(id, msg) => {
                    if let Some(cb) = callbacks.borrow().get(&id) {
                        let ret =
                            ctx.call(cb, &[VVal::Int(id as i64), msg.into_vval()])
                               .expect("no error in cb");

                        if ret.with_s_ref(|s| s == "delete") {
                            remove = Some(id);
                        }
                    }
                },
                Event::PortEnd(id) => {
                    println!("DEBUG: Removed cb {}", id);

                    if let Some(port) = ports.borrow_mut().remove(&id) {
                        port.join();
                    }

                    remove = Some(id);
                },
                Event::LogErr(err) => {
                    eprintln!("Error: {}", err);
                },
            }

            if let Some(remove_id) = remove {
                callbacks.borrow_mut().remove(&remove_id);
            }
        } else {
            break;
        }
    }
}

/*

WLambda API Experiments:


!li_id = dn:wsmp:listen "0.0.0.0:18444";

on li_id {!(id, msg) = @;
    # id is a pair for internal routing!
    dn:send li_id => id $["my:reply", 1, 2];
};

!cli_id = dn:wsmp:connect "192.168.2.10:18444";
on cli_id {!msg = _;
    dn:send cli_id $["reply"];
};

!p_id = dn:process:connect :line $["wlambda"];


!pdate_id = dn:process:oneshot :line:ro $["date"];

dn:send pdate_id $n;
on pdate_id {!date = @;
    std:displayln "DATE:" date;
};

!t_id       = dn:timer:one_shot :ms => 1000 $[:timer1];
!wkup_t_id  = dn:timer:interval :ms => 1000 $[:wakeup];

!last_con_id = $n;
!:global on_msg = {!(id, con_id, msg) = @;
    ? id == srv_id {
        match msg.0
            "eval" => {
                .last_con_id = con_id;
                dn:send p_id "std:displayln 10 + 20";
            };
        break[];
    };
    ? id == cli_id {
        break[];
    };
    ? id == p_id {
        dn:send $p(srv_id, last_con_id) $["eval:result", msg];
    };
};





*/

//fn main() {
//    use tcp_csv_msg_connection::{Event, EventCtx, Msg};
//
//    let (e_tx, e_rx) = EventCtx::new_queue();
//
//    let mut con = tcp_csv_msg_connection::TCPCSVConnection::new(12, e_tx.clone());
//    con.connect("127.0.0.1:18444");
//
//    let my_name       = "test run";
//    let my_version    = "0.1-alpha";
//    let proto_version = "1";
//
//    let mut logged_in  = false;
//    let mut hello_recv = false;
//    let mut hello_sent = false;
//
//    let mut srv = tcp_csv_msg_connection::TCPCSVServer::new(100000, e_tx);
//    srv.start_listener("0.0.0.0:18431");
//
//    // TODO: measure time since last ping sent, if above => send ping
//    // TODO: measure time since last "ok ping", if above => reconnect
//
//    loop {
//        let evctx = e_rx.recv().expect("no error");
//        if evctx.user_id > 100000 {
//            srv.check_event_to_handle(&evctx);
//        }
//
//        let id = evctx.user_id;
//        let ev = evctx.event;
//
//
//        println!("EVENT: {:?}", ev);
//        match ev {
//            Event::ConnectionAvailable => {
//                logged_in = false;
//                con.send(Msg::Hello(vec![
//                    my_name.to_string(),
//                    my_version.to_string(),
//                    proto_version.to_string()
//                ]));
//            },
//            Event::ConnectionLost => {
//            },
//            Event::RecvMessage(msg) => {
//                match &msg {
//                    Msg::Hello(args) => {
//                        println!("Handshake with: {:?}", args);
//                        hello_recv = true;
//                        if hello_recv && hello_sent {
//                            logged_in = true;
//                            println!("Logged in!");
//                        }
//                    },
//                    Msg::Ok(cmd) => {
//                        match &cmd[..] {
//                            "hello" => {
//                                hello_sent = true;
//                                if hello_recv && hello_sent {
//                                    logged_in = true;
//                                    println!("Logged in!");
//                                }
//                            },
//                            _ => (),
//                        }
//                    },
//                    Msg::Payload(p_cmd, args, payload) => {
//                        if logged_in {
//                            println!("RECV. PAYLOAD [{}]: {:?}", p_cmd, payload);
//                        }
//                    },
//                    Msg::Ping => {
//                        // NOP, handled by ok.
//                    },
//                    Msg::Quit => {
//                    },
//                    Msg::Direct(args) => {
//                        if logged_in {
//                            println!("DIRECT: {:?}", args);
//                        }
//                    },
//                    Msg::Error(_) => {
//                    },
//                }
//
//                if let Some(resp) = msg.ok_response() {
//                    con.send(resp);
//                }
//            },
//            Event::SentMessage => {
//            },
//            Event::ReaderConnectionAvailable => {
//                println!("reader there!");
//            },
//            Event::LogErr(err) => {
//                println!("** error: {}", err);
//            },
//            Event::LogInf(err) => {
//                println!("** info: {}", err);
//            },
//            Event::ConnectError(err) => {
//                println!("Connect error: {}", err);
//            },
//        }
//    }
//
//
//    {
////        let mut c = tcp_csv_msg_connection::TCPCSVConnection::new();
////
////        c.connect();
////        loop {
////            match c.reader_rx.as_mut().unwrap().recv() {
////                Ok(tcp_csv_msg_connection::Event::IncomingMessage(msg)) => {
////                    if msg == "quit\n" {
////                        break;
////                    }
////                    println!("RECV: {}", msg);
////                },
////                Ok(tcp_csv_msg_connection::Event::Connected(m)) => {
////                    println!("CONNECTED {}", m);
////                    for i in 0..100000 {
////                        c.send("AAAAAAAAAAAAAAAAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
////                    }
////                },
////                Ok(it) => {
////                    println!("RECV: {:?}", it);
////                },
////                Err(e) => {
////                    println!("ERROR: {}", e);
////                }
////            }
////        }
////
////        c.shutdown();
//    }
//
//
////    let mut dc = DetachedCommand::start("wlambda", &[]).expect("X");
////
////    dc.send_str("10 + 20\n");
////    loop {
////        match dc.poll() {
////            Ok(()) => {
////                if dc.stderr_available() {
////                    println!("SE: {}", dc.recv_stderr());
////                }
////                if dc.stdout_available() {
////                    println!("SO: {}", dc.recv_stdout());
////                }
////            },
////            Err(err) => {
////                println!("stdout: [{}]", dc.recv_stdout());
////                println!("stderr: [{}]", dc.recv_stderr());
////                println!("Error in poll: {:?}", err);
////                break;
////            },
////        }
////    }
////
////    dc.shutdown();
//}

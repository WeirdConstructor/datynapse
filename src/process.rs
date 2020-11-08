use std::thread::{JoinHandle, spawn};
use std::sync::mpsc::*;
use std::sync::{Arc, Mutex};
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::io::Write;
use std::io::BufRead;

use crate::event::*;
use crate::msg;

pub enum Cmd {
    Input(Vec<u8>),
    Kill,
}

pub enum CmdProtocol {
    LineBased,
    WSMP,
}

pub fn kill_child(child: &Arc<Mutex<std::process::Child>>) {
    if let Ok(mut child) = child.lock() {
        child.kill().is_ok();
    }
}

pub fn start_process(event_tx: Sender<Event>,
                     rx: Receiver<Cmd>,
                     id: u64,
                     cmd_exe: &str,
                     args: &[&str],
                     protocol: CmdProtocol) -> JoinHandle<()>
{
    let mut cmd = Command::new(cmd_exe);
    let cmd_exe = cmd_exe.to_string();

    for a in args.iter() {
        cmd.arg(a);
    }

    spawn(move || {

        cmd.stdout(Stdio::piped())
           .stderr(Stdio::null())
           .stdin(Stdio::piped());

        let mut child =
            match cmd.spawn() {
                Ok(child) => child,
                Err(e) => {
                    event_tx.send(
                        Event::LogErr(
                            format!("Error starting '{}': {}",
                                    cmd_exe, e))).is_ok();
                    return;
                },
            };

        let stdin  = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        let child      = Arc::new(Mutex::new(child));
        let stop       = Arc::new(Mutex::new(false));
        let cmd_exe_w  = cmd_exe.to_string();
        let child_w    = child.clone();
        let event_tx_w = event_tx.clone();
        let stop_w     = stop.clone();

        let writer_thread = spawn(move || {
            let mut bw = std::io::BufWriter::new(stdin);

            loop {
                match stop_w.lock() {
                    Ok(stop) => { if *stop { return; } },
                    Err(_)   => { return; },
                }

                match rx.recv_timeout(Duration::from_millis(1000)) {
                    Ok(Cmd::Kill)     => { kill_child(&child_w); return; },
                    Ok(Cmd::Input(s)) => {
                        if let Err(e) = bw.write_all(&s) {
                            event_tx_w.send(
                                Event::LogErr(
                                    format!("Writing to '{}' failed: {}",
                                        cmd_exe_w, e))).is_ok();
                            return;
                        }

                        if let Err(e) = bw.flush() {
                            event_tx_w.send(
                                Event::LogErr(
                                    format!("Flushing to '{}' failed: {}",
                                        cmd_exe_w, e))).is_ok();
                            return;
                        }
                    },
                    Err(RecvTimeoutError::Timeout) => {
                        // nop
                    },
                    Err(RecvTimeoutError::Disconnected) => {
                        std::thread::sleep(
                            std::time::Duration::from_millis(250));
                        return;
                    },
                }
            }
        });


        match protocol {
            CmdProtocol::LineBased => {
                let mut br = std::io::BufReader::new(stdout);
                loop {
                    let mut line = String::new();
                    match br.read_line(&mut line) {
                        Ok(s) => {
                            event_tx.send(
                                Event::Message(id, Msg::Str(None, line))).is_ok();
                            if s == 0 { break; }
                        },
                        Err(e) => {
                            event_tx.send(
                                Event::LogErr(
                                    format!("Error executing '{}': {}",
                                            cmd_exe, e))).is_ok();
                            kill_child(&child);
                            break;
                        }
                    }
                }
            },
            CmdProtocol::WSMP => {
                let mut stdout = std::io::BufReader::new(stdout);

                loop {
                    let mut rd = msg::MessageReader::new();
                    let mut try_again = true;

                    while try_again {
                        try_again = false;

                        match rd.read_msg(&cmd_exe, &mut stdout) {
                            Ok(msg) => {
                                match msg {
                                    msg::Msg::Quit => { kill_child(&child); },
                                    msg::Msg::Ok(_)    => { },
                                    msg::Msg::Ping     => { },
                                    msg::Msg::Hello(_) => { },
                                    msg::Msg::Error(e) => {
                                        event_tx.send(
                                            Event::LogErr(
                                                format!("Error from '{}': {:?}",
                                                        cmd_exe, e))).is_ok();
                                    },
                                    msg => {
                                        if let Some(msg) = Msg::from_msg(msg) {
                                            event_tx.send(
                                                Event::Message(id, msg)).is_ok();
                                        }
                                    },
                                }
                            },
                            Err(msg::ReadMsgError::Timeout) => {
                                // nop
                            },
                            Err(msg::ReadMsgError::TryAgain) => {
                                try_again = true;
                            },
                            Err(e) => {
                                event_tx.send(
                                    Event::LogErr(
                                        format!("Error executing '{}': {:?}",
                                            cmd_exe, e))).is_ok();
                                kill_child(&child);
                            },
                        }
                    }
                }
            },
        }

        if let Ok(mut stop) = stop.lock() {
            *stop = true;
        }

        match child.lock() {
            Ok(mut child) =>
                match child.wait() {
                    Ok(status) => {
                        event_tx.send(
                            Event::Message(id,
                                Msg::Direct(
                                    vec![
                                        "end".to_string(),
                                        status.code().unwrap_or(-1).to_string()])))
                                .is_ok();
                        event_tx.send(Event::DeleteCallback(id)).is_ok();
                    },
                    Err(e) => {
                        event_tx.send(
                            Event::LogErr(
                                format!("Error waiting for end of '{}': {}",
                                        cmd_exe, e))).is_ok();
                    },
                },
            Err(e) => {
                event_tx.send(
                    Event::LogErr(
                        format!("Error lock child waiting for end of '{}': {}",
                                cmd_exe, e))).is_ok();
            },
        }

        writer_thread.join();
    })
}

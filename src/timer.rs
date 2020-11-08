use crate::event::*;
use std::sync::mpsc::*;
use std::thread::{JoinHandle, spawn};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub enum Timer {
    Timeouted,
    Oneshot(u64, u64),
    Interval(u64, u64, u64),
}

pub fn start_timer_thread(event_tx: Sender<Event>, rx: Receiver<Timer>) -> JoinHandle<()> {
    spawn(move || {
        let mut list = vec![];
        let mut free_list = vec![];

        let max_timeout = 100000000;
        let mut min_timeout = 100000000;

        loop {
            let before_recv = Instant::now();
            match rx.recv_timeout(Duration::from_millis(min_timeout)) {
                Ok(timer) => {
                    if free_list.is_empty() {
                        list.push(timer);
                    } else {
                        let idx = free_list.pop().unwrap();
                        list[idx] = timer;
                    }
                },
                Err(RecvTimeoutError::Timeout) => {
                    ()
                },
                Err(RecvTimeoutError::Disconnected) => {
                    break;
                },
            }

            let passed_time = before_recv.elapsed().as_millis() as u64;

            min_timeout = max_timeout;

            for (idx, timer) in list.iter_mut().enumerate() {
                match timer {
                    Timer::Timeouted => (),
                    Timer::Oneshot(id, tout) => {
                        if *tout > passed_time {
                            let new_tout = *tout - passed_time;
                            if new_tout < min_timeout {
                                min_timeout = new_tout;
                            }

                            *timer = Timer::Oneshot(*id, new_tout);
                        } else {
                            // TODO: Handle send error!
                            event_tx.send(Event::Timeout(*id)).is_ok();
                            event_tx.send(Event::PortEnd(*id)).is_ok();

                            *timer = Timer::Timeouted;
                            free_list.push(idx);
                        }
                    },
                    Timer::Interval(id, tout, orig_tout) => {
                        if *tout > passed_time {
                            let new_tout = *tout - passed_time;
                            if new_tout < min_timeout {
                                min_timeout = new_tout;
                            }

                            *timer = Timer::Interval(*id, new_tout, *orig_tout);
                        } else {
                            // TODO: Handle send error!
                            event_tx.send(Event::Timeout(*id));

                            let new_tout = *orig_tout;
                            if new_tout < min_timeout {
                                min_timeout = new_tout;
                            }

                            *timer = Timer::Interval(*id, *orig_tout, *orig_tout);
                        }
                    },
                }
            }

            //d// println!("RR: {:?}", list);
        }
    })
}


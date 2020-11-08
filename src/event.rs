use wlambda::VVal;
use crate::msg;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Msg {
    JSON(Option<Vec<String>>, String),
    MsgPack(Option<Vec<String>>, Vec<u8>),
    Bytes(Option<Vec<String>>, Vec<u8>),
    Str(Option<Vec<String>>, String),
    Direct(Vec<String>),
}

impl Msg {
    pub fn from_msg(m: msg::Msg) -> Option<Self> {
        match m {
            msg::Msg::Quit          => None,
            msg::Msg::Ok(_)         => None,
            msg::Msg::Ping          => None,
            msg::Msg::Hello(_)      => None,
            msg::Msg::Error(_)      => None,
            msg::Msg::Direct(args)  => Some(Msg::Direct(args)),
            msg::Msg::Payload(typ, args, payload) => {
                Some(match typ {
                    "json" => {
                        match String::from_utf8(payload) {
                            Ok(s)  => Msg::JSON(args, s),
                            Err(e) => Msg::JSON(args,
                                String::from_utf8_lossy(e.as_bytes())
                                    .to_string()),
                        }
                    },
                    "msgpack" => Msg::MsgPack(args, payload),
                    _         => Msg::Bytes(args, payload),
                })
            },
        }
    }

    pub fn into_vval(self) -> VVal {
        match self {
            Msg::JSON(args, s) => {
                let v =
                    VVal::from_json(&s)
                        .unwrap_or_else(|e| VVal::err_msg(&e));
                if let Some(args) = args {
                    let va =
                        VVal::vec_mv(
                            args.into_iter().map(
                                |a| VVal::new_str_mv(a)).collect());
                    va.push(v);
                    va
                } else {
                    v
                }
            },
            Msg::MsgPack(args, b) => {
                let v =
                    VVal::from_msgpack(&b)
                        .unwrap_or_else(|e| VVal::err_msg(&e));
                if let Some(args) = args {
                    let va =
                        VVal::vec_mv(
                            args.into_iter().map(
                                |a| VVal::new_str_mv(a)).collect());
                    va.push(v);
                    va
                } else {
                    v
                }
            },
            Msg::Bytes(args, b) => {
                if let Some(args) = args {
                    let v =
                        VVal::vec_mv(
                            args.into_iter().map(
                                |a| VVal::new_str_mv(a)).collect());
                    v.push(VVal::new_byt(b));
                    v
                } else {
                    VVal::new_byt(b)
                }
            },
            Msg::Str(args, s) => {
                if let Some(args) = args {
                    let v =
                        VVal::vec_mv(
                            args.into_iter().map(
                                |a| VVal::new_str_mv(a)).collect());
                    v.push(VVal::new_str_mv(s));
                    v
                } else {
                    VVal::new_str_mv(s)
                }
            },
            Msg::Direct(args) => {
                VVal::vec_mv(
                    args.into_iter().map(
                        |a| VVal::new_str_mv(a)).collect())
            },
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Event {
    PortEnd(u64),
    Timeout(u64),
    Message(u64, Msg),
    LogErr(String),
}

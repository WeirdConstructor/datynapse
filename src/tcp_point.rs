#[derive(Debug, PartialEq, Copy, Clone)]
struct Id {
    SrvClient(usize, usize),
    Connection(usize),
}

enum Error {
    Unknown,
}

#[derive(Debug, PartialEq, Copy, Clone)]
enum Message {
    Connected,
    Disconnted,
    Data,
    SendQueueEmpty,
}

type TCPRes = Result<Message, Error>;

struct TCPPoint {
    // server vector with vector of connections
    // direct connection vector with connection info (for reconnect)
    // internal message queue
    // global atom for stopping everything?!
}

impl TCPPoint {
    fn listen_on(port: u16) -> Id {
        // Make server socket
        // Listen on port
        // Start accepter thread
        // For new sockets, create a connection and store it in the global vector
        Id::SrvClient(0, 0)
    }

    fn connect_to(host: String, port: u16, timeout: std::time::Duration) -> Id {
        Id::Connection(0)
    }

    fn next() -> Option<(Id, TCPRes)> {
        None
    }
}

struct SyncHandle {
}

impl SyncHandle {
    fn send(data: &[u8]) -> TCPRes {
        Err(Error::Unknown)
    }

    fn read_some() -> TCPRes {
        Err(Error::Unknown)
    }

    fn wait_connect() -> TCPRes {
        Err(Error::Unknown)
    }

    fn wait_disconnect() -> TCPRes {
        Err(Error::Unknown)
    }
}

/*
let tcppoint = TCPPoint::new();

let id_srv = tcppoint.listen_on(30232);
let id_srv2 = tcppoint.listen_on(302);

let id = tcppoint.connect_to("127.0.0.1", 2131, TimeoutMS(100));
let id2 = tcppoint.connect_to("192.168.3.3", 2131, TimeoutMS(100));

tcppoint.send(id, "hello\r\n");

while let Some((id, msg)) = tcppoint.next() {
    if id.is_srv() {
        ...
    }

    match msg {
        TCPSendQueueEmpty() => {
        },
        TCPConnected() => {
            let epinfo = tcppoint.get_endpoint_info(id);
        },
        TCPDisconnected() => {
        },
        TCPError(err) => {
        },
        TCPData(data) => {
            if data.contains(x) {
                tcppoint.send("quit\r\n");
                tcppoint.disconnect_after_send(id);
                tcppoint.disconnect_now(id);
            }
        },
    }
}

let id = tcppoint.connect_to(...., Timeout(100));
let sync_con = tcppoint.sync_handle(id);

sync_con.send("foo"); // Waits until error, disconnect or queue empty

sync_con.read_some(); // waits until data, disconnect or queue empty

sync_con.wait_disconnect(); // waits until disconnect or timeout

sync_con.wait_connect();    // waits until connect or timeout

*/

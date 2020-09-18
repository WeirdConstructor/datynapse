#[derive(Debug, PartialEq, Copy, Clone)]
struct TCPId {
    SrvClient(usize, usize),
    Connection(usize),
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

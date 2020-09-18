#[derive(Debug, PartialEq, Copy, Clone)]
struct TCPId {
    SrvClient(usize),
    Connection(usize),
}


/*
let tcppoint = TCPPoint::new();

let id_srv = tcppoint.listen_on(30232);
let id_srv2 = tcppoint.listen_on(302);

let id = tcppoint.connect_to("127.0.0.1", 2131);
let id2 = tcppoint.connect_to("192.168.3.3", 2131);

tcppoint.send(id, "hello\r\n");

while let Some(msg) = tcppoint.next() {
    match msg {
        TCPSrv(id, msg) => {
            match msg {
                TCPConnected(id) => {
                    let epinfo = tcppoint.get_endpoint_info(id);
                    tcppoint.send(id, "hello\r\n");
                },
                TCPDisconnected(id) => {
                },
                TCPError(id, err) => {
                },
                TCPData(id, data) => {
                    if data.contains(x) {
                        tcppoint.send("quit\r\n");
                        tcppoint.disconnect_after_send(id);
                        tcppoint.disconnect_now(id);
                    }
                },
            }
        },
        TCPConnected(id) => {
            let epinfo = tcppoint.get_endpoint_info(id);
        },
        TCPDisconnected(id) => {
        },
        TCPError(id, err) => {
        },
        TCPData(id, data) => {
        },
    }
}





*/

use std::collections::HashMap;

use quiche::ConnectionId;

use super::QuicInnerConn;

#[allow(unused)]
pub struct QuicListener {
    conns: HashMap<ConnectionId<'static>, QuicInnerConn>,
}

use std::collections::HashMap;

use quiche::ConnectionId;

use super::QuicConnState;

#[allow(unused)]
pub struct QuicListener {
    conns: HashMap<ConnectionId<'static>, QuicConnState>,
}

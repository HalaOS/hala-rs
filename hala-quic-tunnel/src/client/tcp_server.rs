use std::collections::HashMap;

use std::io::Write;

use bytes::{Buf, BytesMut};
use mio::{
    event::Event,
    net::{TcpListener, TcpStream},
    Interest, Poll, Token,
};

use crate::utils::{interrupted, read_buf, would_block};

use super::TunnelClientConfig;

pub struct TcpServer {
    /// Client tcp incoming listener
    tcp_listener: TcpListener,
    /// The collection of incoming tcp streams.
    tcp_conns: HashMap<Token, TcpStream>,
    /// Backward data cache buf.
    backward_bufs: HashMap<Token, BytesMut>,
    /// TCP recv buff
    tcp_recv_buf: usize,
}

impl TcpServer {
    /// Create tunnel client with providing config
    pub fn new(poll: &mut Poll, token: Token, config: &TunnelClientConfig) -> anyhow::Result<Self> {
        let mut tcp_listener = TcpListener::bind(config.get_listen_addr()?)?;

        poll.registry()
            .register(&mut tcp_listener, token, Interest::READABLE)?;

        Ok(Self {
            tcp_listener,
            tcp_conns: Default::default(),
            backward_bufs: Default::default(),
            tcp_recv_buf: config.tcp_recv_buf,
        })
    }

    pub fn poll_accept<G: FnMut() -> Token>(
        &mut self,
        poll: &mut Poll,
        mut token_generator: G,
    ) -> anyhow::Result<bool> {
        loop {
            let (mut connection, _) = match self.tcp_listener.accept() {
                Ok(incoming) => incoming,
                Err(err) => {
                    if would_block(&err) {
                        return Ok(true);
                    } else {
                        return Err(err.into());
                    }
                }
            };

            log::info!("incoming new tcp connection {:?}", connection);

            let token = token_generator();

            poll.registry().register(
                &mut connection,
                token,
                Interest::READABLE.add(Interest::WRITABLE),
            )?;

            self.tcp_conns.insert(token, connection);
        }
    }

    /// Check if this event raise by tcp connection which managed by this object.
    pub fn is_raised_by_tcp_conn(&self, event: &Event) -> bool {
        self.tcp_conns.contains_key(&event.token())
    }

    pub fn poll_read(
        &mut self,
        poll: &mut Poll,
        token: Token,
        buf: &mut BytesMut,
    ) -> anyhow::Result<()> {
        let mut connection_closed = false;

        let  conn = self.tcp_conns.get_mut(&token).expect("Call is_raised_by_tcp_conn to check if event raised by tcp connection which managed by this object");

        // We can (maybe) read from the connection.
        loop {
            match read_buf(conn, buf) {
                Ok(0) => {
                    // Reading 0 bytes means the other side has closed the
                    // connection or is done writing, then so are we.
                    connection_closed = true;
                    break;
                }
                Ok(_) => {
                    buf.reserve(self.tcp_recv_buf);
                }
                // Would block "errors" are the OS's way of saying that the
                // connection is not actually ready to perform this I/O operation.
                Err(ref err) if would_block(err) => break,
                Err(ref err) if interrupted(err) => continue,
                // Other errors we'll consider fatal.
                Err(err) => return Err(err.into()),
            }
        }

        // connection closed , clear connection resources.
        if connection_closed {
            poll.registry().deregister(conn)?;

            self.tcp_conns.remove(&token);

            self.backward_bufs.remove(&token);
        }

        Ok(())
    }

    pub fn poll_write<B: Buf>(
        &mut self,
        poll: &mut Poll,
        token: Token,
        mut buf: B,
    ) -> anyhow::Result<()> {
        let  conn = self.tcp_conns.get_mut(&token).expect("Call is_raised_by_tcp_conn to check if event raised by tcp connection which managed by this object");

        loop {
            match conn.write(buf.chunk()) {
                Ok(n) => {
                    buf.advance(n);

                    log::trace!(
                        "Write data len({}) to conn {:?}, remaining {}",
                        n,
                        conn,
                        buf.remaining()
                    );

                    poll.registry().register(
                        conn,
                        token,
                        Interest::READABLE.add(Interest::WRITABLE),
                    )?;
                    return Ok(());
                }
                Err(ref err) if would_block(err) => {
                    log::trace!(
                        "Try write data to conn {:?} would_block, try next time",
                        conn
                    );
                    return Ok(());
                }
                Err(ref err) if interrupted(err) => {
                    log::trace!(
                        "Try write data to conn {:?} interrupted, retry immediately",
                        conn
                    );
                }
                Err(err) => return Err(err.into()),
            }
        }
    }
}

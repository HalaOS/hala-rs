mod config;

pub mod quice_client;
pub mod tcp_server;

use std::{cell::RefCell, rc::Rc};

use bytes::BytesMut;
pub use config::Config as TunnelClientConfig;
use mio::{Events, Poll, Token};

use crate::utils::interrupted;

use self::tcp_server::TcpServer;

const ACCEPTOR: Token = Token(0);

/// The quic tunnel client side.
pub struct TunnelClient {
    config: TunnelClientConfig,
    /// io event poll
    poll: Poll,
    /// Tcp server object.
    tcp_server: TcpServer,
    /// The token generator seed.
    token_next: Rc<RefCell<usize>>,
}

impl TunnelClient {
    /// Create tunnel client with providing config
    pub fn new(config: TunnelClientConfig) -> anyhow::Result<Self> {
        // Create a poll instance.
        let mut poll = Poll::new()?;

        let tcp_server = TcpServer::new(&mut poll, ACCEPTOR, &config)?;

        Ok(Self {
            poll,
            config,
            tcp_server,
            token_next: Default::default(),
        })
    }

    /// run client events loop
    pub fn run_loop(&mut self) -> anyhow::Result<()> {
        // Create storage for events.
        let mut events = Events::with_capacity(self.config.max_poll_events);

        loop {
            if let Err(err) = self.poll.poll(&mut events, None) {
                if interrupted(&err) {
                    continue;
                }
                return Err(err.into());
            }

            self.handle_events(&events)?;
        }
    }

    fn handle_events(&mut self, events: &Events) -> anyhow::Result<()> {
        for event in events.iter() {
            match event.token() {
                ACCEPTOR => {
                    let token_next = self.token_next.clone();

                    self.tcp_server.poll_accept(&mut self.poll, || {
                        *token_next.borrow_mut() += 1;

                        Token(*token_next.borrow())
                    })?;
                }
                _ => {
                    if self.tcp_server.is_raised_by_tcp_conn(event) {
                        if event.is_readable() {
                            let mut buf = BytesMut::with_capacity(1024);

                            self.tcp_server
                                .poll_read(&mut self.poll, event.token(), &mut buf)?;
                        }

                        if event.is_writable() {
                            let buf = vec![0 as u8; 20];

                            self.tcp_server.poll_write(
                                &mut self.poll,
                                event.token(),
                                buf.as_ref(),
                            )?;
                        }
                    } else {
                        todo!("handle udp endpoint events")
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;

    #[test]
    fn test_bytes_mut() {
        let mut buf = BytesMut::zeroed(1024);

        assert_eq!(buf.len(), 1024);

        assert_eq!(buf.capacity(), 1024);

        buf.as_mut()[0] = b'a';

        buf.resize(2048, 0);

        assert_eq!(buf.len(), 2048);
    }
}

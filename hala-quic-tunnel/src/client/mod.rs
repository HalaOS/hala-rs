use std::{collections::HashMap, io::Write};

use bytes::BytesMut;
use mio::{
    event::Event,
    net::{TcpListener, TcpStream},
    Events, Interest, Poll, Token,
};

mod config;
mod quice_client;
mod tcp_server;

pub use config::Config as TunnelClientConfig;

use crate::utils::{interrupted, would_block};

const ACCEPTOR: Token = Token(0);

/// The quic tunnel client side.
pub struct TunnelClient {
    config: TunnelClientConfig,
    /// io event poll
    poll: Poll,
    /// Client tcp incoming listener
    tcp_listener: TcpListener,
    /// The next token generator seed, start from 1
    token_next: usize,
    /// The collection of incoming tcp streams.
    tcp_conns: HashMap<Token, TcpStream>,
    /// Backward data cache buf.
    backward_bufs: HashMap<Token, BytesMut>,
}

impl TunnelClient {
    /// Create tunnel client with providing config
    pub fn new(config: TunnelClientConfig) -> anyhow::Result<Self> {
        // Create a poll instance.
        let poll = Poll::new()?;

        let mut tcp_listener = TcpListener::bind(config.get_listen_addr()?)?;

        poll.registry()
            .register(&mut tcp_listener, ACCEPTOR, Interest::READABLE)?;

        Ok(Self {
            poll,
            config,
            tcp_listener,
            token_next: 0,
            tcp_conns: Default::default(),
            backward_bufs: Default::default(),
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
                ACCEPTOR => self.handle_accept()?,
                _ => self.handle_conn_event(event)?,
            }
        }

        Ok(())
    }

    fn handle_accept(&mut self) -> anyhow::Result<()> {
        loop {
            let (mut connection, _) = match self.tcp_listener.accept() {
                Ok(incoming) => incoming,
                Err(err) => {
                    if would_block(&err) {
                        return Ok(());
                    } else {
                        return Err(err.into());
                    }
                }
            };

            log::info!("incoming new tcp connection {:?}", connection);

            let token = self.next_token();

            self.poll.registry().register(
                &mut connection,
                token,
                Interest::READABLE.add(Interest::WRITABLE),
            )?;

            self.tcp_conns.insert(token, connection);
        }
    }

    fn handle_conn_event(&mut self, event: &Event) -> anyhow::Result<()> {
        let done = if let Some(conn) = self.tcp_conns.get_mut(&event.token()) {
            if event.is_writable() {
                if let Some(buf) = self.backward_bufs.remove(&event.token()) {
                    match conn.write(&buf) {
                        Ok(n) if n < buf.len() => {}
                        Ok(_) => {}
                    }
                }
            }
            true
        } else {
            false
        };

        if done {
            if let Some(mut connection) = self.tcp_conns.remove(&event.token()) {
                self.poll.registry().deregister(&mut connection)?;
            }
        }

        Ok(())
    }

    fn next_token(&mut self) -> Token {
        self.token_next += 1;

        Token(self.token_next)
    }
}

use std::io::Write;
use std::{collections::HashMap, net::SocketAddr};

use bytes::{Buf, BytesMut};
use mio::{
    event::Event,
    net::{TcpListener, TcpStream},
    Interest, Poll, Token,
};

use crate::utils::{interrupted, read_buf, would_block};

fn default_tcp_recv_buf() -> usize {
    4096
}

#[derive(Debug, Default)]
#[cfg_attr(feature = "config_serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde_clap", derive(clap::Parser))]
#[cfg_attr(feature = "serde_clap", command(author, version, about, long_about = None))]
pub struct TcpServerConfig {
    /// tcp server listening address default("0.0.0.0:12345"),
    #[cfg_attr(feature = "serde_clap", arg(short, long))]
    pub listen: Option<String>,

    /// tcp stream maximum recv buf.
    #[cfg_attr(feature = "serde_clap", arg(short, long, default_value_t = 4096))]
    #[cfg_attr(feature = "config_serde", serde(default = "default_tcp_recv_buf"))]
    pub tcp_recv_buf: usize,
}

impl TcpServerConfig {
    /// Get local tcp listening address.
    pub fn get_listen_addr(&self) -> anyhow::Result<SocketAddr> {
        Ok(self
            .listen
            .clone()
            .unwrap_or("0.0.0.0:12345".to_owned())
            .parse()?)
    }
}

pub struct TcpServer {
    token: Token,
    /// Server config
    config: TcpServerConfig,
    /// Client tcp incoming listener
    tcp_listener: TcpListener,
    /// The collection of incoming tcp streams.
    tcp_conns: HashMap<Token, TcpStream>,
}

impl TcpServer {
    /// Create tunnel client with providing config
    pub fn new(poll: &mut Poll, token: Token, config: TcpServerConfig) -> anyhow::Result<Self> {
        let mut tcp_listener = TcpListener::bind(config.get_listen_addr()?)?;

        poll.registry()
            .register(&mut tcp_listener, token, Interest::READABLE)?;

        Ok(Self {
            token,
            tcp_listener,
            tcp_conns: Default::default(),
            config,
        })
    }

    pub fn poll_close(&mut self, poll: &mut Poll) -> anyhow::Result<()> {
        poll.registry()
            .reregister(&mut self.tcp_listener, self.token, Interest::READABLE)?;

        for (_, stream) in self.tcp_conns.iter_mut() {
            poll.registry().deregister(stream)?;
        }

        self.tcp_conns.clear();

        Ok(())
    }

    /// Close [`TcpStream`] handled by this server by `token`
    pub fn poll_close_conn(&mut self, poll: &mut Poll, token: Token) -> anyhow::Result<()> {
        if let Some(mut stream) = self.tcp_conns.remove(&token) {
            poll.registry().deregister(&mut stream)?;
        }

        Ok(())
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
                    buf.reserve(self.config.tcp_recv_buf);
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
        }

        Ok(())
    }

    pub fn poll_write<B: Buf>(
        &mut self,
        poll: &mut Poll,
        token: Token,
        mut buf: B,
    ) -> anyhow::Result<bool> {
        let  conn = self.tcp_conns.get_mut(&token).expect("Call is_raised_by_tcp_conn to check if event raised by tcp connection which managed by this object");

        loop {
            match conn.write(buf.chunk()) {
                Ok(n) if n < buf.remaining() => {
                    buf.advance(n);

                    log::trace!(
                        "Write data len({}) to conn {:?}, remaining {}",
                        n,
                        conn,
                        buf.remaining()
                    );

                    poll.registry()
                        .reregister(conn, token, Interest::WRITABLE)?;

                    continue;
                }

                Ok(n) => {
                    buf.advance(n);

                    log::trace!("Write data len({}) to conn {:?}", n, conn);

                    poll.registry()
                        .reregister(conn, token, Interest::WRITABLE)?;
                    return Ok(true);
                }
                Err(ref err) if would_block(err) => {
                    log::trace!(
                        "Try write data to conn {:?} would_block, try next time",
                        conn
                    );
                    return Ok(false);
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

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        io::{Read, Write},
        net::{SocketAddr, TcpStream},
        thread::{sleep, spawn},
        time::Duration,
    };

    use bytes::BytesMut;
    use mio::{Events, Poll, Token};

    use crate::utils::interrupted;

    use super::{TcpServer, TcpServerConfig};

    fn echo_client(message: &str) {
        // let config: TcpServerConfig =
        //     serde_json::from_str(include_str!("tcp_config.json")).unwrap();

        let mut stream =
            TcpStream::connect("127.0.0.1:1812".parse::<SocketAddr>().unwrap()).unwrap();

        stream.write_all(message.as_bytes()).unwrap();

        let mut echo = String::new();

        stream.read_to_string(&mut echo).unwrap();

        log::info!("recv echo: {}", echo);

        assert_eq!(echo, message);
    }

    #[test]
    fn test_tcp_server() {
        _ = pretty_env_logger::try_init();

        let mut poll = Poll::new().unwrap();

        let config: TcpServerConfig =
            serde_json::from_str(include_str!("tcp_config.json")).unwrap();

        let mut tcp_server = TcpServer::new(&mut poll, Token(0), config).unwrap();

        let mut events = Events::with_capacity(1024);

        let mut next_token = 0;

        let mut bufs = HashMap::new();

        spawn(|| {
            sleep(Duration::from_secs(2));
            echo_client("hello world");
        });

        loop {
            if let Err(err) = poll.poll(&mut events, None) {
                if interrupted(&err) {
                    continue;
                }

                panic!("poll events failed: {err:?}");
            }

            for event in events.iter() {
                log::info!("handle event {:?}", event);

                match event.token() {
                    Token(0) => {
                        tcp_server
                            .poll_accept(&mut poll, || {
                                next_token += 1;

                                Token(next_token)
                            })
                            .unwrap();
                    }
                    _ => {
                        assert!(tcp_server.is_raised_by_tcp_conn(event));

                        if event.is_readable() {
                            let mut buf = BytesMut::with_capacity(1024);

                            tcp_server
                                .poll_read(&mut poll, event.token(), &mut buf)
                                .unwrap();

                            log::info!(
                                "recv message '{}' from {:?}",
                                String::from_utf8_lossy(&buf),
                                event.token()
                            );

                            if tcp_server
                                .poll_write(&mut poll, event.token(), buf.as_ref())
                                .unwrap()
                            {
                                tcp_server
                                    .poll_close_conn(&mut poll, event.token())
                                    .unwrap();

                                return;
                            }

                            bufs.insert(event.token(), buf);
                        }

                        // echo
                        if event.is_writable() {
                            if let Some(buf) = bufs.get(&event.token()) {
                                tcp_server
                                    .poll_write(&mut poll, event.token(), buf.as_ref())
                                    .unwrap();

                                tcp_server
                                    .poll_close_conn(&mut poll, event.token())
                                    .unwrap();

                                return;
                            }
                        }
                    }
                }
            }
        }
    }
}

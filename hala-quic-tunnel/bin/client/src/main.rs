mod args;
use args::*;
use clap::Parser;
use mio::{net::TcpListener, Events, Interest, Poll, Token};

const SERVER: Token = Token(0);

fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let args = Args::parse();

    let listen_addr = args.get_listen_addr()?;

    // Create a poll instance.
    let mut poll = Poll::new()?;

    // Create storage for events.
    let mut events = Events::with_capacity(args.max_poll_events);

    let mut server = TcpListener::bind(args.get_listen_addr()?)?;

    poll.registry()
        .register(&mut server, SERVER, Interest::READABLE)?;

    Ok(())
}

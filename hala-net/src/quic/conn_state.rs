//! The quic connection state matchine implementation.

use std::{
    cell::RefCell,
    fmt::Debug,
    io,
    marker::PhantomData,
    ops,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use event_map::{
    locks::{WaitableLocker, WaitableSpinMutex},
    EventMap, WaitableEventMap,
};

use hala_io_util::{timeout_with, ContextPoller, GlobalContextPoller, LocalContextPoller};
use quiche::{ConnectionId, SendInfo};

use crate::errors::into_io_error;

pub struct QuicConnState {
    /// quiche connection state machine.
    quiche_conn: quiche::Connection,
    /// The timestamp of last ping packet sent.
    send_ack_eliciting_instant: Instant,
    /// The interval between two ping packets.
    ping_timeout: Duration,
}

/// The io event variants for quic connection state mache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum QuicConnStateEvent {
    /// This event notify listener that this state machine is now readable.
    Readable(ConnectionId<'static>),
    /// This event notify listener that this state machine is now writable.
    Writable(ConnectionId<'static>),

    /// This event notify listener that one stream of this state machine is now readable.
    StreamReadable(ConnectionId<'static>, u64),

    /// This event notify listener that one stream of this state machine is now writable.
    StreamWritable(ConnectionId<'static>, u64),
}

/// The state matchine for quic connection.
pub struct WaitableQuicConnStateMaker<State, Mediator, Context> {
    /// The [`QuicConnState`] instance with lock protected.
    state: State,
    /// The [`EventMap`] instance.
    mediator: Mediator,
    /// The source id of this connection.
    scid: ConnectionId<'static>,
    /// The destination id of this connection.
    dcid: ConnectionId<'static>,
    ///
    _marked: PhantomData<Context>,
}

impl<State, Mediator, Context> Debug for WaitableQuicConnStateMaker<State, Mediator, Context> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "QuicConnState, scid={:?}, dcid={:?}",
            self.scid, self.dcid
        )
    }
}

impl<State, Mediator, Context> Clone for WaitableQuicConnStateMaker<State, Mediator, Context>
where
    State: Clone,
    Mediator: Clone,
{
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            mediator: self.mediator.clone(),
            scid: self.scid.clone(),
            dcid: self.dcid.clone(),
            _marked: PhantomData,
        }
    }
}

pub trait WaitableQuicConnStateBuilder {
    type State;
    type Mediator;

    /// Create new `QuicConnState` instance.
    ///
    /// # Parameters
    ///
    /// - `scid` The source id of the underly quic connection.
    /// - `dcid` The destination id of the underly quic connection.
    fn new(
        scid: ConnectionId<'static>,
        dcid: ConnectionId<'static>,
        raw: Self::State,
        mediator: Self::Mediator,
    ) -> Self;
}

impl<State, Mediator, Context> WaitableQuicConnStateBuilder
    for WaitableQuicConnStateMaker<State, Mediator, Context>
where
    State: WaitableLocker<Value = QuicConnState>,
    Mediator: WaitableEventMap<E = QuicConnStateEvent>,
{
    type Mediator = Mediator;
    type State = State;

    fn new(
        scid: ConnectionId<'static>,
        dcid: ConnectionId<'static>,
        raw: State,
        mediator: Mediator,
    ) -> Self {
        Self {
            state: raw,
            mediator,
            scid,
            dcid,
            _marked: PhantomData,
        }
    }
}

impl<State, Mediator, Context> WaitableQuicConnStateMaker<State, Mediator, Context>
where
    State: WaitableLocker<Value = QuicConnState>,
    for<'a> State::WaitableGuard<'a>: Unpin,
    Mediator: WaitableEventMap<E = QuicConnStateEvent>,
    Context: ContextPoller,
{
    /// Read a single QUIC packet to be sent to the peer.
    ///
    /// if there is nothing to read, this function will `pending` until the state changes to
    /// [`writable`](QuicConnStateEvent::Writable).
    ///
    ///
    pub async fn read(&self, buf: &mut [u8]) -> io::Result<(usize, SendInfo)> {
        let event = QuicConnStateEvent::Readable(self.scid.clone());

        // Asynchronously lock the [`QuicConnState`]
        let mut state = self.state.async_lock().await;

        loop {
            match state.quiche_conn.send(buf) {
                Ok((send_size, send_info)) => {
                    log::trace!(
                        "{:?} read data, len={}, send_info={:?}",
                        self,
                        send_size,
                        send_info
                    );

                    return Ok((send_size, send_info));
                }
                Err(quiche::Error::Done) => {
                    let send_timeout = state.quiche_conn.timeout();

                    log::trace!("{:?} read data pending, timeout={:?}", self, send_timeout);

                    let wait_fut = async {
                        self.mediator
                            .wait(event.clone(), state)
                            .await
                            .map_err(into_io_error)
                    };

                    let wait_fut_with_timeout =
                        timeout_with(wait_fut, send_timeout, Context::get()?);

                    match wait_fut_with_timeout.await {
                        Ok(s) => {
                            state = s;
                            continue;
                        }
                        Err(err) if err.kind() == io::ErrorKind::TimedOut => {
                            // cancel waiting readable event notify.
                            self.mediator.wait_cancel(&event);
                            // relock state.
                            state = self.state.async_lock().await;

                            if state.send_ack_eliciting_instant.elapsed() > state.ping_timeout {
                                state
                                    .quiche_conn
                                    .send_ack_eliciting()
                                    .map_err(into_io_error)?;

                                log::trace!(
                                    "{:?} ping packet, timout={:?}",
                                    self,
                                    state.ping_timeout,
                                );

                                continue;
                            }

                            state.quiche_conn.on_timeout();

                            log::debug!(
                                "{:?} pending on_timeout, timeout={:?}",
                                self,
                                send_timeout
                            );

                            continue;
                        }
                        Err(err) => {
                            log::error!("{:?} read data pending, err={}", self, err);
                            return Err(into_io_error(err));
                        }
                    }
                }
                Err(err) => {
                    log::error!("{:?} read data, err={}", self, err);

                    return Err(into_io_error(err));
                }
            }
        }
    }
}

/// Define new type of quic connections state machine for multi-thread mode.
pub type WaitableQuicConnState = WaitableQuicConnStateMaker<
    Arc<WaitableSpinMutex<QuicConnState>>,
    Arc<EventMap<QuicConnStateEvent>>,
    GlobalContextPoller,
>;

/// Define new type of quic connections state machine for single-thread mode.
pub type WaitableLocalQuicConnState = WaitableQuicConnStateMaker<
    Rc<RefCell<QuicConnState>>,
    Rc<EventMap<QuicConnStateEvent>>,
    LocalContextPoller,
>;

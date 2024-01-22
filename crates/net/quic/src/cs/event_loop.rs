use std::sync::Arc;

use futures::{
    channel::mpsc::{Receiver, Sender},
    SinkExt, StreamExt,
};
use hala_io::{
    bytes::{Bytes, BytesMut},
    ReadBuf,
};
use hala_udp::{PathInfo, UdpGroup};
use quiche::{RecvInfo, SendInfo};

use super::QuicConnCmd;

pub(super) async fn quic_conn_send_loop(
    label: String,
    mut receiver: Receiver<(Bytes, SendInfo)>,
    udp_socket: Arc<UdpGroup>,
) {
    log::trace!("{}, start send loop", label);

    while let Some((buf, send_info)) = receiver.next().await {
        log::trace!("{}, send, delay={:?}", label, send_info.at.elapsed());

        match udp_socket
            .send_to_on_path(
                &buf,
                PathInfo {
                    from: send_info.from,
                    to: send_info.to,
                },
            )
            .await
        {
            Ok(len) => {
                assert_eq!(len, buf.len());
            }
            Err(err) => {
                log::trace!("{}, send loop stopped by error, {}", label, err);

                return;
            }
        }
    }

    log::trace!("{}, send loop stopped", label);
}

pub(super) async fn quic_conn_recv_loop(
    label: String,
    mut sender: Sender<QuicConnCmd>,
    udp_socket: Arc<UdpGroup>,
) {
    log::trace!("{}, start recv loop", label);

    loop {
        let mut buf = ReadBuf::with_capacity(1370);

        match udp_socket.recv_from(buf.as_mut()).await {
            Ok((read_size, path_info)) => {
                if sender
                    .send(QuicConnCmd::Recv {
                        buf: buf.into_bytes_mut(Some(read_size)),
                        recv_info: RecvInfo {
                            from: path_info.from,
                            to: path_info.to,
                        },
                    })
                    .await
                    .is_err()
                {
                    // conn had been dropped.
                    break;
                }
            }
            Err(err) => {
                log::trace!("{}, recv loop stopped by error, {}", label, err);

                return;
            }
        }
    }

    log::trace!("{:?}, recv loop stopped", label);
}

pub(super) async fn quic_listener_recv_loop(
    label: String,
    mut sender: Sender<(BytesMut, RecvInfo)>,
    udp_socket: Arc<UdpGroup>,
) {
    log::trace!("{}, start recv loop", label);

    loop {
        let mut buf = ReadBuf::with_capacity(1370);

        match udp_socket.recv_from(buf.as_mut()).await {
            Ok((read_size, path_info)) => {
                if sender
                    .send((
                        buf.into_bytes_mut(Some(read_size)),
                        RecvInfo {
                            from: path_info.from,
                            to: path_info.to,
                        },
                    ))
                    .await
                    .is_err()
                {
                    // conn had been dropped.
                    break;
                }
            }
            Err(err) => {
                log::trace!("{}, recv loop stopped by error, {}", label, err);

                return;
            }
        }
    }

    log::trace!("{:?}, recv loop stopped", label);
}

use futures::{io::copy, AsyncRead, AsyncWrite, AsyncWriteExt};
use rgnix::{ConnId, Session};

pub async fn tunnel_copy<R, W>(target: &str, session: Session, reader: R, mut writer: W)
where
    R: AsyncRead,
    W: AsyncWrite + Unpin,
{
    let r = match copy(reader, &mut writer).await {
        Ok(bytes) => {
            log::info!(target: target, "{:?}, EOF with transferred {}", session.id, bytes);
            Ok(())
        }
        Err(err) => {
            log::info!(target: target, "{:?}, broken with error, {}", session.id, err);
            Err(err)
        }
    };

    close_writer(target, &session.id, &mut writer).await;

    session.closed_with(r);
}

async fn close_writer<W>(target: &str, id: &ConnId<'static>, writer: &mut W)
where
    W: AsyncWrite + Unpin + ?Sized,
{
    match writer.close().await {
        Ok(_) => {
            log::trace!(target: target, "{:?}, try close write stream", id);
        }
        Err(err) => {
            log::trace!(target: target, "{:?}, try close write stream, with error, {}", id, err);
        }
    };
}

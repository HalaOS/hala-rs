use std::io;

use bytes::BufMut;

pub fn would_block(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::WouldBlock
}

pub fn interrupted(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::Interrupted
}

/// Try to read data from an `Read` into an implementation of the [`BufMut`] trait.
pub fn read_buf<R: io::Read, B: BufMut>(read: &mut R, buf: &mut B) -> io::Result<usize> {
    if !buf.has_remaining_mut() {
        return Err(io::Error::new(
            io::ErrorKind::OutOfMemory,
            "There's no more capacity in the `buf` to write new data",
        ));
    }

    let chunk_mut = buf.chunk_mut();

    let chunk_mut = unsafe { &mut *(chunk_mut as *mut _ as *mut [u8]) };

    let read_size = read.read(chunk_mut)?;

    unsafe {
        buf.advance_mut(read_size);
    }

    Ok(read_size)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use bytes::BytesMut;

    use super::read_buf;

    #[test]
    fn test_read_buf() {
        let mut cursor = Cursor::new(vec![11; 15]);

        let mut buf = BytesMut::with_capacity(7);

        read_buf(&mut cursor, &mut buf).unwrap();

        assert_eq!(buf.len(), 7);
        assert_eq!(buf.as_ref(), vec![11; 15][0..7].as_ref());

        assert_eq!(buf.capacity(), 7);

        assert_eq!(read_buf(&mut cursor, &mut buf).unwrap(), 8);
    }
}

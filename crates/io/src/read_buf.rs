use bytes::{Buf, BufMut, Bytes, BytesMut};

pub fn as_bytes_mut<B>(buf: &mut B) -> &mut [u8]
where
    B: BufMut,
{
    let dst = buf.chunk_mut();

    unsafe { &mut *(dst as *mut _ as *mut [u8]) }
}

pub struct ReadBuf {
    bytes: BytesMut,
}

impl ReadBuf {
    pub fn with_capacity(capacity: usize) -> Self {
        ReadBuf {
            bytes: BytesMut::with_capacity(capacity),
        }
    }

    pub fn chunk_mut(&mut self) -> &mut [u8] {
        let dst = self.bytes.chunk_mut();

        unsafe { &mut *(dst as *mut _ as *mut [u8]) }
    }

    pub fn filled(&mut self, advance: usize) {
        unsafe {
            self.bytes.advance_mut(advance);
        }
    }

    pub fn into_bytes_mut(mut self, advance: Option<usize>) -> BytesMut {
        if let Some(advance) = advance {
            self.filled(advance);
        }

        self.bytes
    }

    pub fn into_bytes(self, advance: Option<usize>) -> Bytes {
        self.into_bytes_mut(advance).into()
    }

    pub fn chunk(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    pub fn split_to(&mut self, at: usize) -> BytesMut {
        self.bytes.split_to(at)
    }

    /// Advance the internal cursor of the Buf
    ///
    /// The next call to `chunk()` will return a slice starting `cnt` bytes
    /// further into the underlying buffer.
    pub fn advance(&mut self, cnt: usize) {
        self.bytes.advance(cnt)
    }
}

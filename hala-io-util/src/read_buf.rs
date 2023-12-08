use bytes::BufMut;

pub fn as_bytes_mut<B>(buf: &mut B) -> &mut [u8]
where
    B: BufMut,
{
    let dst = buf.chunk_mut();

    unsafe { &mut *(dst as *mut _ as *mut [u8]) }
}

use bytes::{Bytes, BytesMut};
use futures::{io::Cursor, AsyncRead, AsyncReadExt};
use hala_io::ReadBuf;

use crate::parse::{ParseError, ParseResult};

/// Http body reader created by http parsers, see mod [`parse`](crate::parse)  for more information.
#[derive(Debug)]
pub struct BodyReader<S> {
    /// Unparsed data cached by the parser when parsing http headers
    cached: Bytes,

    /// The underlying transport layer data stream
    stream: S,
}

impl<S> BodyReader<S> {
    /// Create new `BodyReader` instance from `(Bytes,S)`
    pub fn new(cached: Bytes, stream: S) -> Self {
        Self { cached, stream }
    }
}

impl<S> BodyReader<S>
where
    S: AsyncRead + Unpin,
{
    /// Consume `BodyReader` and create an [`AsyncRead`] instance.
    pub fn into_read(self) -> impl AsyncRead {
        Cursor::new(self.cached).chain(self.stream)
    }

    /// Consume `BodyReader` into [`BytesMut`].
    ///
    /// Use `max_body_len` to limit memory buf usage, which may be useful for server-side code.
    pub async fn into_bytes(self, max_body_len: usize) -> ParseResult<BytesMut> {
        let mut stream = self.into_read();

        let mut read_buf = ReadBuf::with_capacity(max_body_len);

        loop {
            let chunk_mut = read_buf.chunk_mut();

            // Checks if the parsing buf is overflowing.
            if chunk_mut.len() == 0 {
                return Err(ParseError::ParseBufOverflow(max_body_len));
            }

            let read_size = stream.read(chunk_mut).await?;

            if read_size == 0 {
                break;
            }

            read_buf.filled(read_size);
        }

        Ok(read_buf.into_bytes_mut(None))
    }

    /// Deserialize an instance of type `T` from http json format body.
    ///
    /// The maximum body length is limited to `4096` bytes,
    /// use [`from_json_with`](Self::from_json_with) instead if you want to use other values.
    #[cfg(feature = "json")]
    pub async fn from_json<T>(self) -> crate::parse::ParseResult<T>
    where
        for<'a> T: serde::de::Deserialize<'a>,
    {
        self.from_json_with(4096).await
    }

    /// Deserialize an instance of type `T` from http json format body.
    ///
    /// Use `max_body_len` to limit memory buf usage, which may be useful for server-side code.
    #[cfg(feature = "json")]
    pub async fn from_json_with<T>(self, max_body_len: usize) -> crate::parse::ParseResult<T>
    where
        for<'a> T: serde::de::Deserialize<'a>,
    {
        let buf = self.into_bytes(max_body_len).await?;

        Ok(serde_json::from_slice(&buf)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hala_io::test::io_test;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Mock {
        a: i32,
        b: String,
    }

    #[hala_test::test(io_test)]
    async fn test_from_json() {
        let mock = Mock {
            a: 10,
            b: "hello".to_string(),
        };

        let mut json_data = Bytes::from(serde_json::to_string_pretty(&mock).unwrap());

        let body_reader = BodyReader::new(
            json_data.split_to(json_data.len() / 2),
            Cursor::new(json_data),
        );

        let mock2 = body_reader.from_json::<Mock>().await.unwrap();

        assert_eq!(mock, mock2);
    }
}

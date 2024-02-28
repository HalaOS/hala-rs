use std::io;

use bytes::BytesMut;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite};
use hala_io::ReadBuf;
use http::{
    header::{InvalidHeaderName, InvalidHeaderValue},
    uri::InvalidUri,
    HeaderMap, HeaderName, HeaderValue, Method, Request, Uri, Version,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Token length is zero.")]
    Token,

    #[error("Uri token length is zero.")]
    UriToken,

    #[error("Input stream ")]
    Eof,

    #[error("Stream ops error: {0}")]
    Io(#[from] io::Error),

    #[error("Unsupport http method")]
    InvalidMethod,

    #[error("Unsupport http uri: {0}")]
    InvalidUri(#[from] InvalidUri),

    #[error("Unsupport http version.")]
    InvalidVersion,

    #[error("New line token error, expect \r\n")]
    Newline,

    #[error("Header name is empty.")]
    HeaderName,

    #[error("{0}")]
    InvalidHeaderName(#[from] InvalidHeaderName),

    #[error("Header value is empty.")]
    HeaderValue,

    #[error("{0}")]
    InvalidHeaderValue(#[from] InvalidHeaderValue),

    #[error("{0}")]
    HttpError(#[from] http::Error),
}

impl From<ParseError> for io::Error {
    fn from(value: ParseError) -> Self {
        match value {
            ParseError::Io(io) => io,
            _ => io::Error::new(io::ErrorKind::Other, value),
        }
    }
}

/// Type alias of [`Result<T, ParseError>`]
pub type ParseResult<T> = Result<T, ParseError>;

/// The http [`Request`](http::Request), [`Response`](http::Response) parser config.
pub struct ParserConfig {
    header_parse_max_buf: usize,
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            header_parse_max_buf: 2048,
        }
    }
}

/// Request parser inner state machine.
#[derive(Default)]
struct RequestParsing {
    /// The state sequence number of this state machine. the value range is [0..4).
    state: usize,
    /// The request's method
    method: Option<Method>,

    /// The request's URI
    uri: Option<Uri>,

    /// The request's version
    version: Option<Version>,

    /// The request's headers
    headers: HeaderMap<HeaderValue>,
}

/// Inner func [`RequestParser::skip_newline`] result variant.
enum SkipNewLine {
    /// do nothing
    None,
    /// skip one newline token.
    One,
    /// This is a sequence of two line breaks, indicating that processing
    /// of the current paragraph has been completed.
    Break,
    /// newline token is incomplete.
    Incomplete,
}

/// Inner func [`RequestParser::skip_newline`] result variant.
enum ParseHeader {
    /// parse one header name / value pair.
    One,
    /// the parsed is the last one item of header field.
    Last,
    /// incomplete header item
    Incomplete,
}

/// The parser which parse incoming `stream` into [`Request`](http::Request).
pub struct RequestBuilder<S> {
    /// tag for log information output.
    debug_info: String,
    /// http protocol parse config.
    #[allow(unused)]
    parser_config: ParserConfig,
    /// head parse buf.
    read_buf: ReadBuf,
    /// read http request packet from this stream.
    stream: S,
    /// the parser statemachine.
    parsing: RequestParsing,
}

impl<S> RequestBuilder<S> {
    /// Consume `stream` and generate a request parser.
    pub fn new(debug_info: &str, stream: S) -> Self {
        Self::new_with(debug_info, stream, Default::default())
    }

    /// Create new `RequestParser` with customer [`ParserConfig`]
    pub fn new_with(debug_info: &str, stream: S, parser_config: ParserConfig) -> Self {
        let read_buf = ReadBuf::with_capacity(parser_config.header_parse_max_buf);

        Self {
            debug_info: debug_info.into(),
            parser_config,
            read_buf,
            stream,
            parsing: Default::default(),
        }
    }
}

impl<S> RequestBuilder<S>
where
    S: AsyncWrite + AsyncRead + Unpin,
{
    ///  Parse and generate [`Request`] object.
    pub async fn parse(mut self) -> ParseResult<Request<S>> {
        'outer: loop {
            let read_size = self.stream.read(self.read_buf.chunk_mut()).await?;

            // reach stream EOF.
            if read_size == 0 {
                return Err(ParseError::Eof);
            }

            self.read_buf.filled(read_size);

            'inner: while self.read_buf.chunk().len() > 0 {
                match self.parsing.state {
                    // parse method.
                    0 => {
                        self.parse_method()?;
                        self.skip_spaces();
                    }
                    // parse uri
                    1 => {
                        if self.parse_uri()? {
                            self.skip_spaces();
                        } else {
                            // incomplete uri.
                            break 'inner;
                        }
                    }
                    // parse version: HTTP/x.x
                    2 => {
                        self.parse_version()?;
                    }
                    3 => match self.parse_headers()? {
                        ParseHeader::Last => {
                            break 'outer;
                        }
                        ParseHeader::Incomplete => {
                            break 'inner;
                        }
                        _ => {}
                    },
                    _ => {
                        panic!("Not here, inner error.");
                    }
                };
            }
        }

        let mut builder = http::request::Builder::new()
            .method(self.parsing.method.unwrap())
            .uri(self.parsing.uri.unwrap())
            .version(self.parsing.version.unwrap());

        for (k, v) in self.parsing.headers.drain() {
            if let Some(k) = k {
                builder = builder.header(k.clone(), v.clone());
            }
        }

        Ok(builder.body(self.stream)?)
    }

    fn skip_spaces(&mut self) {
        self.read_buf.split_to(skip_spaces(self.read_buf.chunk()));
    }

    fn skip_newline(&mut self) -> ParseResult<SkipNewLine> {
        match skip_newline(self.read_buf.chunk())? {
            SkipNewLine::One => {
                self.read_buf.split_to(2);
                Ok(SkipNewLine::One)
            }
            SkipNewLine::Break => {
                self.read_buf.split_to(4);

                Ok(SkipNewLine::Break)
            }
            status => Ok(status),
        }
    }

    fn parse_headers(&mut self) -> ParseResult<ParseHeader> {
        self.skip_spaces();

        match self.skip_newline()? {
            SkipNewLine::Break => return Ok(ParseHeader::Last),
            SkipNewLine::Incomplete => return Ok(ParseHeader::Incomplete),
            _ => {}
        }

        // parse name.
        let buf = self.read_buf.chunk();

        let mut header_name = None;

        for (offset, c) in buf.iter().cloned().enumerate() {
            if c == b':' {
                if offset == 0 {
                    return Err(ParseError::HeaderName);
                }

                let buf = self.read_buf.split_to(offset + 1);

                header_name = Some(HeaderName::from_bytes(&buf[0..(buf.len() - 1)])?);

                break;
            }
        }

        // Incomplete header name.
        if header_name.is_none() {
            return Ok(ParseHeader::Incomplete);
        }

        // skip space between ':' and value.
        self.skip_spaces();

        // parse value.
        let buf = self.read_buf.chunk();

        let mut header_value = None;

        for (offset, c) in buf.iter().cloned().enumerate() {
            if c == b'\r' {
                if offset == 0 {
                    header_value = Some(HeaderValue::from_static(""));
                } else {
                    let mut buf = self.read_buf.split_to(offset);

                    // trim suffix spaces.
                    trim_suffix_spaces(&mut buf);

                    header_value = Some(HeaderValue::from_maybe_shared(buf)?);
                }

                break;
            }
        }

        // Incomplete header value.
        if header_name.is_none() {
            return Ok(ParseHeader::Incomplete);
        }

        self.parsing
            .headers
            .insert(header_name.unwrap(), header_value.unwrap());

        Ok(ParseHeader::One)
    }

    fn parse_method(&mut self) -> ParseResult<()> {
        let token = parse_token(self.read_buf.chunk())?;

        if token.is_none() {
            return Ok(());
        }

        let token = token.unwrap();

        match Method::from_bytes(token) {
            Ok(method) => {
                self.parsing.method = Some(method);
                self.parsing.state += 1;
                self.read_buf.split_to(token.len());
                Ok(())
            }
            Err(_) => {
                log::error!(
                    "{}, Parse http header error, invalid method {:?}",
                    self.debug_info,
                    token
                );

                Err(ParseError::InvalidMethod)
            }
        }
    }

    fn parse_uri(&mut self) -> ParseResult<bool> {
        let token = parse_uri(self.read_buf.chunk())?;

        if token.is_none() {
            return Ok(false);
        }

        let token = token.unwrap();

        let token_buf = self.read_buf.split_to(token.len());

        match Uri::from_maybe_shared(token_buf) {
            Ok(uri) => {
                self.parsing.uri = Some(uri);
                self.parsing.state += 1;

                Ok(true)
            }
            Err(err) => Err(err.into()),
        }
    }

    fn parse_version(&mut self) -> ParseResult<()> {
        let token = parse_version(self.read_buf.chunk())?;

        if token.is_none() {
            return Ok(());
        }

        let token = token.unwrap();

        self.read_buf.split_to(8);

        self.parsing.version = Some(token);

        self.parsing.state += 1;

        Ok(())
    }
}

#[inline]
fn parse_token(bytes: &[u8]) -> ParseResult<Option<&[u8]>> {
    assert!(bytes.len() > 0, "Check buf len before call parse_token");

    for (offset, b) in bytes.iter().cloned().enumerate() {
        if b == b' ' {
            if offset == 0 {
                return Err(ParseError::Token);
            }

            return Ok(Some(&bytes[0..offset]));
        }
    }

    // The token data is not complete, return `None`
    Ok(None)
}

#[inline]
fn skip_spaces(buf: &[u8]) -> usize {
    for (offset, b) in buf.iter().cloned().enumerate() {
        if b != b' ' && b != b'\t' {
            return offset;
        }
    }

    buf.len()
}

#[inline]
fn trim_suffix_spaces(buf: &mut BytesMut) {
    for (offset, c) in buf.iter().rev().cloned().enumerate() {
        if c != b' ' && c != b'\t' {
            if offset > 0 {
                _ = buf.split_off(buf.len() - offset);
            }

            break;
        }
    }
}

#[inline]
fn _skip_newline(buf: &[u8]) -> ParseResult<SkipNewLine> {
    if buf.len() > 1 {
        if b"\r\n" == &buf[..2] {
            return Ok(SkipNewLine::One);
        }
    } else if buf.len() > 0 {
        match buf[0] {
            b'\n' => {
                return Err(ParseError::Newline);
            }
            b'\r' => {
                return Ok(SkipNewLine::Incomplete);
            }
            _ => {}
        }
    }

    Ok(SkipNewLine::None)
}

#[inline]
fn skip_newline(buf: &[u8]) -> ParseResult<SkipNewLine> {
    match _skip_newline(buf)? {
        SkipNewLine::One => match _skip_newline(&buf[2..])? {
            SkipNewLine::One => Ok(SkipNewLine::Break),
            SkipNewLine::None | SkipNewLine::Incomplete => Ok(SkipNewLine::One),
            SkipNewLine::Break => {
                panic!("not here");
            }
        },
        status => Ok(status),
    }
}

#[inline]
fn parse_uri(buf: &[u8]) -> ParseResult<Option<&[u8]>> {
    assert!(buf.len() > 0);

    for (offset, b) in buf.iter().cloned().enumerate() {
        if b == b' ' || b == b'\t' {
            if offset == 0 {
                return Err(ParseError::UriToken);
            }
            return Ok(Some(&buf[0..offset]));
        }
    }

    // The token data is not complete, return `None`
    Ok(None)
}

#[inline]
fn parse_version(buf: &[u8]) -> ParseResult<Option<Version>> {
    if buf.len() >= 8 {
        return match &buf[0..8] {
            b"HTTP/0.9" => Ok(Some(Version::HTTP_09)),
            b"HTTP/1.0" => Ok(Some(Version::HTTP_10)),
            b"HTTP/1.1" => Ok(Some(Version::HTTP_11)),
            b"HTTP/2.0" => Ok(Some(Version::HTTP_2)),
            b"HTTP/3.0" => Ok(Some(Version::HTTP_3)),
            _ => Err(ParseError::InvalidVersion),
        };
    }

    Ok(None)
}

#[cfg(test)]
mod tests {

    use futures::io::Cursor;
    use hala_io::test::io_test;
    use http::{Method, Request, Version};

    use super::*;

    async fn parse_request(buf: &[u8]) -> ParseResult<Request<Cursor<Vec<u8>>>> {
        RequestBuilder::new("test", Cursor::new(buf.to_vec()))
            .parse()
            .await
    }

    async fn parse_request_test<F>(buf: &[u8], f: F)
    where
        F: FnOnce(Request<Cursor<Vec<u8>>>),
    {
        let request = parse_request(buf).await.expect("parse request failed.");

        f(request)
    }

    async fn expect_request_partial_parse(buf: &[u8]) {
        let error = parse_request(buf).await.expect_err("");
        if let ParseError::Eof = error {
        } else {
            panic!("Expect eof, but got {:?}", error);
        }
    }

    async fn expect_request_empty_method(buf: &[u8]) {
        let error = parse_request(buf).await.expect_err("");
        if let ParseError::Token = error {
        } else {
            panic!("Expect token, but got {:?}", error);
        }
    }

    #[hala_test::test(io_test)]
    async fn request_tests() {
        parse_request_test(b"GET / HTTP/1.1\r\n\r\n", |req| {
            assert_eq!(req.method(), Method::GET);
            assert_eq!(req.uri().to_string(), "/");
            assert_eq!(req.version(), Version::HTTP_11);
            assert_eq!(req.headers().len(), 0);
        })
        .await;

        parse_request_test(b"GET /thing?data=a HTTP/1.1\r\n\r\n", |req| {
            assert_eq!(req.method(), Method::GET);
            assert_eq!(req.uri().to_string(), "/thing?data=a");
            assert_eq!(req.version(), Version::HTTP_11);
            assert_eq!(req.headers().len(), 0);
        })
        .await;

        parse_request_test(b"GET /thing?data=a^ HTTP/1.1\r\n\r\n", |req| {
            assert_eq!(req.method(), Method::GET);
            assert_eq!(req.uri().to_string(), "/thing?data=a^");
            assert_eq!(req.version(), Version::HTTP_11);
            assert_eq!(req.headers().len(), 0);
        })
        .await;

        parse_request_test(
            b"GET / HTTP/1.1\r\nHost: foo.com\r\nCookie: \r\n\r\n",
            |req| {
                assert_eq!(req.method(), Method::GET);
                assert_eq!(req.uri().to_string(), "/");
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(req.headers().len(), 2);
                assert_eq!(
                    req.headers().get("Host").unwrap().to_str().unwrap(),
                    "foo.com"
                );
                assert_eq!(req.headers().get("Cookie").unwrap().to_str().unwrap(), "");
            },
        )
        .await;

        parse_request_test(
            b"GET / HTTP/1.1\r\nHost: \tfoo.com\t \r\nCookie: \t \r\n\r\n",
            |req| {
                assert_eq!(req.method(), Method::GET);
                assert_eq!(req.uri().to_string(), "/");
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(req.headers().len(), 2);
                assert_eq!(
                    req.headers().get("Host").unwrap().to_str().unwrap(),
                    "foo.com"
                );
                assert_eq!(req.headers().get("Cookie").unwrap().to_str().unwrap(), "");
            },
        )
        .await;

        parse_request_test(
            b"GET / HTTP/1.1\r\nUser-Agent: some\tagent\r\n\r\n",
            |req| {
                assert_eq!(req.method(), Method::GET);
                assert_eq!(req.uri().to_string(), "/");
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(req.headers().len(), 1);
                assert_eq!(
                    req.headers().get("User-Agent").unwrap().to_str().unwrap(),
                    "some\tagent"
                );
            },
        )
        .await;

        parse_request_test(
            b"GET / HTTP/1.1\r\nUser-Agent: 1234567890some\tagent\r\n\r\n",
            |req| {
                assert_eq!(req.method(), Method::GET);
                assert_eq!(req.uri().to_string(), "/");
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(req.headers().len(), 1);
                assert_eq!(
                    req.headers().get("User-Agent").unwrap().to_str().unwrap(),
                    "1234567890some\tagent"
                );
            },
        )
        .await;

        parse_request_test(
            b"GET / HTTP/1.1\r\nUser-Agent: 1234567890some\t1234567890agent1234567890\r\n\r\n",
            |req| {
                assert_eq!(req.method(), Method::GET);
                assert_eq!(req.uri().to_string(), "/");
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(req.headers().len(), 1);
                assert_eq!(
                    req.headers().get("User-Agent").unwrap().to_str().unwrap(),
                    "1234567890some\t1234567890agent1234567890"
                );
            },
        )
        .await;

        parse_request_test(
            b"GET / HTTP/1.1\r\nHost: foo.com\r\nUser-Agent: \xe3\x81\xb2\xe3/1.0\r\n\r\n",
            |req| {
                assert_eq!(req.method(), Method::GET);
                assert_eq!(req.uri().to_string(), "/");
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(req.headers().len(), 2);
                assert_eq!(
                    req.headers().get("Host").unwrap().to_str().unwrap(),
                    "foo.com"
                );
                assert_eq!(
                    req.headers().get("User-Agent").unwrap().as_bytes(),
                    b"\xe3\x81\xb2\xe3/1.0"
                );
            },
        )
        .await;

        parse_request_test(b"GET /\\?wayne\\=5 HTTP/1.1\r\n\r\n", |req| {
            assert_eq!(req.method(), Method::GET);
            assert_eq!(req.uri().to_string(), "/\\?wayne\\=5");
            assert_eq!(req.version(), Version::HTTP_11);
            assert_eq!(req.headers().len(), 0);
        })
        .await;

        expect_request_partial_parse(b"GET / HTTP/1.1\r\n\r").await;

        expect_request_partial_parse(b"GET / HTTP/1.1\r\nHost: yolo\r\n").await;

        expect_request_partial_parse(b"GET  HTTP/1.1\r\n\r\n").await;

        expect_request_empty_method(b"  HTTP/1.1\r\n\r\n").await;

        expect_request_empty_method(b" / HTTP/1.1\r\n\r\n").await;
    }
}

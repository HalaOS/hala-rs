use std::io;

use bytes::{Bytes, BytesMut};
use futures::{AsyncRead, AsyncReadExt};
use hala_io::ReadBuf;
use http::{
    header::{InvalidHeaderName, InvalidHeaderValue},
    method::InvalidMethod,
    status::InvalidStatusCode,
    uri::InvalidUri,
    HeaderName, HeaderValue, Method, Request, Response, StatusCode, Uri, Version,
};

use crate::body::BodyReader;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error(transparent)]
    HttpError(#[from] http::Error),

    #[error("Http header parse buf overflow, max={0}")]
    ParseBufOverflow(usize),

    #[error(transparent)]
    IoError(#[from] io::Error),

    #[error("Unable to complete http parsing, reached the end of the stream.")]
    Eof,

    #[error("Miss method field.")]
    Method,

    #[error(transparent)]
    InvalidMethod(#[from] InvalidMethod),

    #[error("Miss uri field.")]
    Uri,

    #[error(transparent)]
    InvalidUri(#[from] InvalidUri),

    #[error("Invalid http version.")]
    Version,

    #[error(transparent)]
    InvalidHeaderName(#[from] InvalidHeaderName),

    #[error(transparent)]
    InvalidHeaderValue(#[from] InvalidHeaderValue),

    #[error(transparent)]
    InvalidStatusCode(#[from] InvalidStatusCode),

    #[cfg(feature = "json")]
    #[error(transparent)]
    SerdeJsonError(#[from] serde_json::Error),
}

impl From<ParseError> for io::Error {
    fn from(value: ParseError) -> Self {
        match value {
            ParseError::IoError(err) => err,
            _ => io::Error::new(io::ErrorKind::Other, value),
        }
    }
}

/// Type alias for parser result.
pub type ParseResult<T> = Result<T, ParseError>;

/// Http packet parse config.
#[derive(Debug)]
pub struct Config {
    /// The max buf len for parsing http headers.
    pub parsing_headers_max_buf: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            parsing_headers_max_buf: 2048,
        }
    }
}

/// Http request packet parser.
///
/// In general, please do not create [`Requester`] directly but use
/// [`parse_request`] or [`parse_request_with`] to parse the stream.
pub struct Requester<S> {
    /// http parser config.
    config: Config,

    /// parser statemachine.
    state: RequestParseState,

    /// The stream from which the request parser read bytes.
    stream: S,

    /// the http `request` object builder.
    builder: Option<http::request::Builder>,
}

impl<S> Requester<S> {
    /// Create new `Requester` with provided [`config`](Config)
    pub fn new_with(stream: S, config: Config) -> Self {
        Self {
            config,
            state: RequestParseState::Method,
            stream,
            builder: Some(http::request::Builder::new()),
        }
    }

    /// Create new `Requester` with provided default config.
    pub fn new(stream: S) -> Self {
        Self::new_with(stream, Default::default())
    }
}

impl<S> Requester<S>
where
    S: AsyncRead + Unpin,
{
    /// Try parse http request header parts and generate [`Request`] object.
    pub async fn parse(mut self) -> ParseResult<Request<BodyReader<S>>> {
        // create header parts parse buffer with capacity to `config.parsing_headers_max_buf`
        let mut read_buf = ReadBuf::with_capacity(self.config.parsing_headers_max_buf);

        'out: while self.state != RequestParseState::Finished {
            let chunk_mut = read_buf.chunk_mut();

            // Checks if the parsing buf is overflowing.
            if chunk_mut.len() == 0 {
                return Err(ParseError::ParseBufOverflow(
                    self.config.parsing_headers_max_buf,
                ));
            }

            let read_size = self.stream.read(chunk_mut).await?;

            // EOF reached.
            if read_size == 0 {
                return Err(ParseError::Eof);
            }

            read_buf.filled(read_size);

            'inner: while read_buf.chunk().len() > 0 {
                match self.state {
                    RequestParseState::Method => {
                        if !self.parse_method(&mut read_buf)? {
                            break 'inner;
                        }
                    }
                    RequestParseState::Uri => {
                        if !self.parse_uri(&mut read_buf)? {
                            break 'inner;
                        }
                    }
                    RequestParseState::Version => {
                        if !self.parse_version(&mut read_buf)? {
                            break 'inner;
                        }
                    }
                    RequestParseState::Headers => {
                        if !self.parse_header(&mut read_buf)? {
                            break 'inner;
                        }
                    }
                    RequestParseState::Finished => break 'out,
                }
            }

            if let RequestParseState::Finished = self.state {
                break;
            }
        }

        let cached = read_buf.into_bytes(None);

        // construct [`Request`]
        Ok(self
            .builder
            .unwrap()
            .body(BodyReader::new(cached, self.stream))?)
    }

    #[inline]
    fn skip_spaces(&mut self, read_buf: &mut ReadBuf) {
        read_buf.split_to(skip_spaces(read_buf.chunk()));
    }

    #[inline]
    fn parse_method(&mut self, read_buf: &mut ReadBuf) -> ParseResult<bool> {
        self.skip_spaces(read_buf);

        if let Some(len) = parse_token(read_buf.chunk()) {
            if len == 0 {
                return Err(ParseError::Method);
            }

            let buf = read_buf.split_to(len);

            self.set_method(Method::from_bytes(&buf)?);

            self.state.next();

            Ok(true)
        } else {
            // Incomplete method token.
            Ok(false)
        }
    }

    #[inline]
    fn parse_uri(&mut self, read_buf: &mut ReadBuf) -> ParseResult<bool> {
        self.skip_spaces(read_buf);

        if let Some(len) = parse_token(read_buf.chunk()) {
            if len == 0 {
                return Err(ParseError::Uri);
            }

            let buf = read_buf.split_to(len);

            self.set_uri(Uri::from_maybe_shared(buf)?);

            self.state.next();

            Ok(true)
        } else {
            // Incomplete method token.
            Ok(false)
        }
    }

    #[inline]
    fn parse_version(&mut self, read_buf: &mut ReadBuf) -> ParseResult<bool> {
        self.skip_spaces(read_buf);

        if let Some(version) = parse_version(read_buf.chunk())? {
            // advance read cursor.
            read_buf.split_to(8);

            self.set_version(version);

            self.state.next();

            Ok(true)
        } else {
            Ok(false)
        }
    }

    #[inline]
    fn parse_header(&mut self, read_buf: &mut ReadBuf) -> ParseResult<bool> {
        match skip_newlines(read_buf) {
            SkipNewLine::Break(_) => {
                self.state.next();
                return Ok(false);
            }
            SkipNewLine::Incomplete => return Ok(false),
            SkipNewLine::One(_) | SkipNewLine::None => {}
        }

        match parse_header(read_buf)? {
            Some((name, value)) => {
                self.set_header(name, value);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    #[inline]
    fn set_method(&mut self, method: Method) {
        self.builder = Some(self.builder.take().unwrap().method(method))
    }

    #[inline]
    fn set_uri(&mut self, uri: Uri) {
        self.builder = Some(self.builder.take().unwrap().uri(uri))
    }

    #[inline]
    fn set_version(&mut self, version: Version) {
        self.builder = Some(self.builder.take().unwrap().version(version))
    }

    #[inline]
    fn set_header(&mut self, name: HeaderName, value: HeaderValue) {
        self.builder = Some(self.builder.take().unwrap().header(name, value))
    }
}

/// Helper function to help parsing stream into [`Request`] instance.
///
/// See [`new_with`](Requester::new) for more information.
pub async fn parse_request<S>(stream: S) -> io::Result<Request<BodyReader<S>>>
where
    S: AsyncRead + Unpin,
{
    Ok(Requester::new(stream).parse().await?)
}

/// Helper function to help parsing stream into [`Request`] instance.
///
/// See [`new_with`](Requester::new_with) for more information.
pub async fn parse_request_with<S>(stream: S, config: Config) -> io::Result<Request<BodyReader<S>>>
where
    S: AsyncRead + Unpin,
{
    Ok(Requester::new_with(stream, config).parse().await?)
}

/// Http response packet parser.
///
/// In general, please do not create [`Requester`] directly but use
/// [`parse_response`] or [`parse_response_with`] to parse the stream.
pub struct Responser<S> {
    /// http parser config.
    config: Config,

    /// parser statemachine.
    state: ResponseParseState,

    /// The stream from which the request parser read bytes.
    stream: S,

    /// the http `response` object builder.
    builder: Option<http::response::Builder>,

    /// the http reason
    reason: Option<Bytes>,
}

impl<S> Responser<S> {
    /// Create new `Requester` with provided [`config`](Config)
    pub fn new_with(stream: S, config: Config) -> Self {
        Self {
            config,
            state: ResponseParseState::Version,
            stream,
            builder: Some(http::response::Builder::new()),
            reason: Some(Bytes::from_static(b"")),
        }
    }

    /// Create new `Requester` with provided default config.
    pub fn new(stream: S) -> Self {
        Self::new_with(stream, Default::default())
    }
}

impl<S> Responser<S>
where
    S: AsyncRead + Unpin,
{
    /// Try parse http request header parts and generate [`Request`] object.
    pub async fn parse(mut self) -> ParseResult<Response<BodyReader<S>>> {
        // create header parts parse buffer with capacity to `config.parsing_headers_max_buf`
        let mut read_buf = ReadBuf::with_capacity(self.config.parsing_headers_max_buf);

        'out: while self.state != ResponseParseState::Finished {
            let chunk_mut = read_buf.chunk_mut();

            // Checks if the parsing buf is overflowing.
            if chunk_mut.len() == 0 {
                return Err(ParseError::ParseBufOverflow(
                    self.config.parsing_headers_max_buf,
                ));
            }

            let read_size = self.stream.read(chunk_mut).await?;

            // EOF reached.
            if read_size == 0 {
                return Err(ParseError::Eof);
            }

            read_buf.filled(read_size);

            'inner: while read_buf.chunk().len() > 0 {
                match self.state {
                    ResponseParseState::Version => {
                        if !self.parse_version(&mut read_buf)? {
                            break 'inner;
                        }
                    }
                    ResponseParseState::StatusCode => {
                        if !self.parse_status_code(&mut read_buf)? {
                            break 'inner;
                        }
                    }
                    ResponseParseState::Reason => {
                        if !self.parse_reason(&mut read_buf)? {
                            break 'inner;
                        }
                    }
                    ResponseParseState::Headers => {
                        if !self.parse_header(&mut read_buf)? {
                            break 'inner;
                        }
                    }
                    ResponseParseState::Finished => break 'out,
                }
            }

            if let ResponseParseState::Finished = self.state {
                break;
            }
        }

        let cached = read_buf.into_bytes(None);

        // construct [`Request`]
        Ok(self
            .builder
            .unwrap()
            .body(BodyReader::new(cached, self.stream))?)
    }

    #[inline]
    fn skip_spaces(&mut self, read_buf: &mut ReadBuf) {
        read_buf.split_to(skip_spaces(read_buf.chunk()));
    }

    #[inline]
    fn parse_status_code(&mut self, read_buf: &mut ReadBuf) -> ParseResult<bool> {
        self.skip_spaces(read_buf);

        match parse_code(read_buf.chunk())? {
            Some(code) => {
                self.set_code(code);

                read_buf.split_to(3);

                self.state.next();

                Ok(true)
            }
            None => Ok(false),
        }
    }

    #[inline]
    fn parse_reason(&mut self, read_buf: &mut ReadBuf) -> ParseResult<bool> {
        self.skip_spaces(read_buf);

        match parse_reason(read_buf.chunk()) {
            Some(len) => {
                let buf = read_buf.split_to(len);

                self.set_reason(buf.freeze());

                self.state.next();

                Ok(true)
            }
            None => Ok(false),
        }
    }

    #[inline]
    fn parse_version(&mut self, read_buf: &mut ReadBuf) -> ParseResult<bool> {
        self.skip_spaces(read_buf);

        if let Some(version) = parse_version(read_buf.chunk())? {
            // advance read cursor.
            read_buf.split_to(8);

            self.set_version(version);

            self.state.next();

            Ok(true)
        } else {
            Ok(false)
        }
    }

    #[inline]
    fn parse_header(&mut self, read_buf: &mut ReadBuf) -> ParseResult<bool> {
        match skip_newlines(read_buf) {
            SkipNewLine::Break(_) => {
                self.state.next();
                return Ok(false);
            }
            SkipNewLine::Incomplete => return Ok(false),
            SkipNewLine::One(_) | SkipNewLine::None => {}
        }

        match parse_header(read_buf)? {
            Some((name, value)) => {
                self.set_header(name, value);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    #[inline]
    fn set_code(&mut self, code: StatusCode) {
        self.builder = Some(self.builder.take().unwrap().status(code))
    }

    #[inline]
    fn set_reason(&mut self, reason: Bytes) {
        self.reason = Some(reason);
    }

    #[inline]
    fn set_version(&mut self, version: Version) {
        self.builder = Some(self.builder.take().unwrap().version(version))
    }

    #[inline]
    fn set_header(&mut self, name: HeaderName, value: HeaderValue) {
        self.builder = Some(self.builder.take().unwrap().header(name, value))
    }
}

/// Helper function to help parsing stream into [`Response`] instance.
///
/// See [`new_with`](Response::new) for more information.
pub async fn parse_response<S>(stream: S) -> io::Result<Response<BodyReader<S>>>
where
    S: AsyncRead + Unpin,
{
    Ok(Responser::new(stream).parse().await?)
}

/// Helper function to help parsing stream into [`Response`] instance.
///
/// See [`new_with`](Response::new_with) for more information.
pub async fn parse_response_with<S>(
    stream: S,
    config: Config,
) -> io::Result<Response<BodyReader<S>>>
where
    S: AsyncRead + Unpin,
{
    Ok(Responser::new_with(stream, config).parse().await?)
}

/// The statemachine of [`Requester`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(unused)]
enum ResponseParseState {
    Version = 1,
    StatusCode = 2,
    Reason = 3,
    Headers = 4,
    Finished = 5,
}

impl ResponseParseState {
    fn next(&mut self) {
        if let ResponseParseState::Finished = self {
            return;
        }

        unsafe { *(self as *mut Self as *mut u8) += 1 }
    }
}

/// The statemachine of [`Requester`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(unused)]
enum RequestParseState {
    Method = 1,
    Uri = 2,
    Version = 3,
    Headers = 4,
    Finished = 5,
}

impl RequestParseState {
    fn next(&mut self) {
        if let RequestParseState::Finished = self {
            return;
        }

        unsafe { *(self as *mut Self as *mut u8) += 1 }
    }
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
fn parse_token(buf: &[u8]) -> Option<usize> {
    for (offset, c) in buf.iter().cloned().enumerate() {
        if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' {
            return Some(offset);
        }
    }

    None
}

#[inline]
fn parse_header_name(buf: &[u8]) -> Option<usize> {
    for (offset, c) in buf.iter().cloned().enumerate() {
        if c == b':' {
            return Some(offset);
        }
    }

    None
}

#[inline]
fn parse_header_value(buf: &[u8]) -> Option<usize> {
    for (offset, c) in buf.iter().cloned().enumerate() {
        if c == b'\r' || c == b'\n' {
            return Some(offset);
        }
    }

    None
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
            _ => Err(ParseError::Version),
        };
    }

    Ok(None)
}

/// result of [`skip_newline`]
enum SkipNewLine {
    /// do nothing
    None,
    /// skip one newline token.
    One(usize),
    /// This is a sequence of two line breaks, indicating that processing
    /// of the current paragraph has been completed.
    Break(usize),
    /// newline token is incomplete.
    Incomplete,
}

#[inline]
fn _skip_newline(buf: &[u8]) -> SkipNewLine {
    if buf.len() > 1 {
        if b"\r\n" == &buf[..2] {
            return SkipNewLine::One(2);
        }

        if b"\n\n" == &buf[..2] {
            return SkipNewLine::Break(2);
        }
    }

    if buf.len() > 0 {
        match buf[0] {
            b'\n' => {
                return SkipNewLine::One(1);
            }
            b'\r' => {
                return SkipNewLine::Incomplete;
            }
            _ => {}
        }
    }

    SkipNewLine::None
}

#[inline]
fn _skip_newlines(buf: &[u8]) -> SkipNewLine {
    let mut offset = 0;
    let mut is_break = false;

    loop {
        match _skip_newline(&buf[offset..]) {
            SkipNewLine::Incomplete | SkipNewLine::None => {
                if is_break {
                    return SkipNewLine::Break(offset);
                }

                if offset > 0 {
                    return SkipNewLine::One(offset);
                }

                return SkipNewLine::None;
            }
            SkipNewLine::One(len) => {
                if offset > 0 {
                    is_break = true;
                }

                offset += len;
            }
            SkipNewLine::Break(len) => {
                is_break = true;
                offset += len;
            }
        }
    }
}

#[inline]
fn skip_newlines(read_buf: &mut ReadBuf) -> SkipNewLine {
    let skip_new_line = _skip_newlines(read_buf.chunk());

    match skip_new_line {
        SkipNewLine::One(len) | SkipNewLine::Break(len) => {
            read_buf.split_to(len);
        }
        _ => {}
    }

    skip_new_line
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
fn parse_reason<'a>(buf: &[u8]) -> Option<usize> {
    for (offset, c) in buf.iter().cloned().enumerate() {
        if c == b'\r' || c == b'\n' {
            return Some(offset);
        }
    }

    None
}

#[inline]
fn parse_code(buf: &[u8]) -> ParseResult<Option<StatusCode>> {
    if buf.len() >= 3 {
        Ok(Some(StatusCode::from_bytes(&buf[..3])?))
    } else {
        Ok(None)
    }
}

fn parse_header(read_buf: &mut ReadBuf) -> ParseResult<Option<(HeaderName, HeaderValue)>> {
    let chunk = read_buf.chunk();

    let mut offset = skip_spaces(chunk);

    let name_offset = offset;

    let name_len = match parse_header_name(&chunk[offset..]) {
        Some(name_len) => name_len,
        None => return Ok(None),
    };

    // advance: name + ':'
    offset += name_len + 1;

    let value_offset = skip_spaces(&chunk[offset..]);

    offset += value_offset;

    let value_len = match parse_header_value(&chunk[offset..]) {
        Some(value_len) => value_len,
        None => return Ok(None),
    };

    read_buf.split_to(name_offset);

    let mut buf = read_buf.split_to(name_len);

    trim_suffix_spaces(&mut buf);

    let header_name = HeaderName::from_bytes(&buf)?;

    read_buf.split_to(value_offset + 1);

    let mut buf = read_buf.split_to(value_len);

    trim_suffix_spaces(&mut buf);

    let header_value = HeaderValue::from_maybe_shared(buf)?;

    Ok(Some((header_name, header_value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_state() {
        let mut state = RequestParseState::Method;

        state.next();

        assert_eq!(state, RequestParseState::Uri);

        state.next();

        assert_eq!(state, RequestParseState::Version);

        state.next();

        assert_eq!(state, RequestParseState::Headers);

        state.next();

        assert_eq!(state, RequestParseState::Finished);

        state.next();

        assert_eq!(state, RequestParseState::Finished);
    }

    use futures::io::Cursor;
    use hala_io::test::io_test;
    use http::{Method, Request, Version};

    async fn parse_request(buf: &[u8]) -> ParseResult<Request<BodyReader<Cursor<Vec<u8>>>>> {
        Requester::new(Cursor::new(buf.to_vec())).parse().await
    }

    async fn parse_request_test<F>(buf: &[u8], f: F)
    where
        F: FnOnce(Request<BodyReader<Cursor<Vec<u8>>>>),
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
        if let ParseError::InvalidMethod(_) = error {
        } else {
            panic!("Expect method error, but got {:?}", error);
        }
    }

    async fn expect_request_empty_uri(buf: &[u8]) {
        let error = parse_request(buf).await.expect_err("");
        if let ParseError::InvalidUri(_) = error {
        } else {
            panic!("Expect uri error, but got {:?}", error);
        }
    }

    async fn parse_response(buf: &[u8]) -> ParseResult<Response<BodyReader<Cursor<Vec<u8>>>>> {
        Responser::new(Cursor::new(buf.to_vec())).parse().await
    }

    async fn parse_response_test<F>(buf: &[u8], f: F)
    where
        F: FnOnce(Response<BodyReader<Cursor<Vec<u8>>>>),
    {
        let request = parse_response(buf).await.expect("parse request failed.");

        f(request)
    }

    async fn expect_response_partial_parse(buf: &[u8]) {
        let error = parse_response(buf).await.expect_err("");
        if let ParseError::Eof = error {
        } else {
            panic!("Expect eof, but got {:?}", error);
        }
    }

    async fn expect_response_version(buf: &[u8]) {
        let error = parse_response(buf).await.expect_err("");
        if let ParseError::Version = error {
        } else {
            panic!("Expect version, but got {:?}", error);
        }
    }

    #[hala_test::test(io_test)]
    async fn response_tests() {
        parse_response_test(b"HTTP/1.1 200 OK\r\n\r\n", |resp| {
            assert_eq!(resp.version(), Version::HTTP_11);
            assert_eq!(resp.status(), StatusCode::OK);
        })
        .await;

        parse_response_test(b"HTTP/1.0 403 Forbidden\nServer: foo.bar\n\n", |resp| {
            assert_eq!(resp.version(), Version::HTTP_10);
            assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        })
        .await;

        parse_response_test(b"HTTP/1.1 200 \r\n\r\n", |resp| {
            assert_eq!(resp.version(), Version::HTTP_11);
            assert_eq!(resp.status(), StatusCode::OK);
        })
        .await;

        parse_response_test(b"HTTP/1.1 200\r\n\r\n", |resp| {
            assert_eq!(resp.version(), Version::HTTP_11);
            assert_eq!(resp.status(), StatusCode::OK);
        })
        .await;

        parse_response_test(b"HTTP/1.1 200\r\nFoo: bar\r\n\r\n", |resp| {
            assert_eq!(resp.version(), Version::HTTP_11);
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(resp.headers().len(), 1);

            assert_eq!(resp.headers().get("Foo").unwrap().to_str().unwrap(), "bar");
        })
        .await;

        parse_response_test(b"HTTP/1.1 200 X\xFFZ\r\n\r\n", |resp| {
            assert_eq!(resp.version(), Version::HTTP_11);
            assert_eq!(resp.status(), StatusCode::OK);
        })
        .await;

        parse_response_test(b"HTTP/1.1 200 \x00\r\n\r\n", |resp| {
            assert_eq!(resp.version(), Version::HTTP_11);
            assert_eq!(resp.status(), StatusCode::OK);
        })
        .await;

        parse_response_test(b"HTTP/1.0 200\nContent-type: text/html\n\n", |resp| {
            assert_eq!(resp.version(), Version::HTTP_10);
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(resp.headers().len(), 1);
            assert_eq!(
                resp.headers()
                    .get("Content-type")
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "text/html"
            );
        })
        .await;

        parse_response_test( b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Credentials : true\r\nBread: baguette\r\n\r\n", |resp| {
            assert_eq!(resp.version(), Version::HTTP_11);
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(resp.headers().len(), 2);
            assert_eq!(
                resp.headers()
                    .get("Access-Control-Allow-Credentials")
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "true"
            );

            assert_eq!(
                resp.headers()
                    .get("Bread")
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "baguette"
            );
        })
        .await;

        expect_response_partial_parse(b"HTTP/1.1").await;

        expect_response_partial_parse(b"HTTP/1.1 200").await;

        expect_response_partial_parse(b"HTTP/1.1 200 OK\r\nServer: yolo\r\n").await;

        expect_response_version(b"\n\nHTTP/1.1 200 OK\n\n").await;
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

        expect_request_empty_uri(b"GET  HTTP/1.1\r\n\r\n").await;

        expect_request_empty_method(b"  HTTP/1.1\r\n\r\n").await;

        expect_request_empty_method(b" / HTTP/1.1\r\n\r\n").await;
    }
}

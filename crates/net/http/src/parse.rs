use std::io;

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
    #[error("Parse token error, offset({0})={1}")]
    ParseToken(usize, u8),

    #[error("Parse uri token error, offset({0})={1}")]
    ParseUriToken(usize, u8),

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
pub struct RequestParser<S> {
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

impl<S> RequestParser<S> {
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

impl<S> RequestParser<S>
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
                        self.parse_uri()?;
                        self.skip_spaces();
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

        todo!()
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
                    return Err(ParseError::HeaderValue);
                }

                let buf = self.read_buf.split_to(offset);

                header_value = Some(HeaderValue::from_maybe_shared(buf)?);

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

    fn parse_uri(&mut self) -> ParseResult<()> {
        let token = parse_uri(self.read_buf.chunk())?;

        if token.is_none() {
            return Ok(());
        }

        let token = token.unwrap();

        let token_buf = self.read_buf.split_to(token.len());

        match Uri::from_maybe_shared(token_buf) {
            Ok(uri) => {
                self.parsing.uri = Some(uri);
                self.parsing.state += 1;

                Ok(())
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

/// Determines if byte is a token char.
///
/// > ```notrust
/// > token          = 1*tchar
/// >
/// > tchar          = "!" / "#" / "$" / "%" / "&" / "'" / "*"
/// >                / "+" / "-" / "." / "^" / "_" / "`" / "|" / "~"
/// >                / DIGIT / ALPHA
/// >                ; any VCHAR, except delimiters
/// > ```
#[inline]
fn is_token(b: u8) -> bool {
    b > 0x1F && b < 0x7F
}

#[inline]
fn parse_token(bytes: &[u8]) -> ParseResult<Option<&[u8]>> {
    assert!(bytes.len() > 0, "Check buf len before call parse_token");

    if !is_token(bytes[0]) {
        return Err(ParseError::ParseToken(0, bytes[0]));
    }

    for (offset, b) in bytes[1..].iter().cloned().enumerate() {
        if b == b' ' {
            return Ok(Some(&bytes[0..(offset + 1)]));
        }

        if !is_token(b) {
            return Err(ParseError::ParseToken(offset + 1, bytes[offset + 1]));
        }
    }

    // The token data is not complete, return `None`
    Ok(None)
}

macro_rules! byte_map {
    ($($flag:expr,)*) => ([
        $($flag != 0,)*
    ])
}

#[inline]
fn skip_spaces(buf: &[u8]) -> usize {
    for (offset, b) in buf.iter().cloned().enumerate() {
        if b != b' ' || b != b'\t' {
            return offset;
        }
    }

    buf.len()
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
        SkipNewLine::One => match _skip_newline(buf)? {
            SkipNewLine::One => Ok(SkipNewLine::Break),
            status => Ok(status),
        },
        status => Ok(status),
    }
}

// ASCII codes to accept URI string.
// i.e. A-Z a-z 0-9 !#$%&'*+-._();:@=,/?[]~^
// TODO: Make a stricter checking for URI string?
static URI_MAP: [bool; 256] = byte_map![
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //  \0                            \n
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //  commands
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    //  \w !  "  #  $  %  &  '  (  )  *  +  ,  -  .  /
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1,
    //  0  1  2  3  4  5  6  7  8  9  :  ;  <  =  >  ?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    //  @  A  B  C  D  E  F  G  H  I  J  K  L  M  N  O
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    //  P  Q  R  S  T  U  V  W  X  Y  Z  [  \  ]  ^  _
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    //  `  a  b  c  d  e  f  g  h  i  j  k  l  m  n  o
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,
    //  p  q  r  s  t  u  v  w  x  y  z  {  |  }  ~  del
    //   ====== Extended ASCII (aka. obs-text) ======
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

#[inline]
fn is_uri_token(b: u8) -> bool {
    URI_MAP[b as usize]
}

#[inline]
fn parse_uri(buf: &[u8]) -> ParseResult<Option<&[u8]>> {
    assert!(buf.len() > 0);

    if !is_uri_token(buf[0]) {
        // First char must be a URI char, it can't be a space which would indicate an empty path.
        return Err(ParseError::ParseUriToken(0, buf[0]));
    }

    for (offset, b) in buf[1..].iter().cloned().enumerate() {
        if b == b' ' {
            return Ok(Some(&buf[0..(offset + 1)]));
        }

        if !is_uri_token(b) {
            return Err(ParseError::ParseUriToken(offset + 1, buf[offset + 1]));
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
mod tests {}

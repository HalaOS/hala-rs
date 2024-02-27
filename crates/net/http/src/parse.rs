use std::io;

use bytes::Bytes;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite};
use hala_io::ReadBuf;
use http::{uri::InvalidUri, HeaderMap, HeaderValue, Method, Request, Uri, Version};
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
}

/// Type alias of [`Result<T, ParseError>`]
pub type ParseResult<T> = Result<T, ParseError>;

/// The http [`Request`](http::Request), [`Response`](http::Response) parser config.
pub struct ParserConfig {
    header_parse_max_buf: usize,
    allow_multiple_spaces_in_request_line_delimiters: bool,
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            header_parse_max_buf: 2048,
            allow_multiple_spaces_in_request_line_delimiters: false,
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

/// The parser which parse incoming `stream` into [`Request`](http::Request).
pub struct RequestParser<S> {
    debug_info: String,
    parser_config: ParserConfig,
    read_buf: ReadBuf,
    stream: S,
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
        loop {
            let read_size = self.stream.read(self.read_buf.chunk_mut()).await?;

            // reach stream EOF.
            if read_size == 0 {
                return Err(ParseError::Eof);
            }

            self.read_buf.filled(read_size);

            while self.read_buf.chunk().len() > 0 {
                match self.parsing.state {
                    // parse method.
                    0 => {
                        self.parse_method()?;
                    }
                    // parse uri
                    1 => {
                        self.parse_uri()?;
                    }
                    2 => {}
                    3 => {}
                    _ => {
                        panic!("Not here, inner error.");
                    }
                };
            }

            self.skip_spaces();
        }
    }

    fn skip_spaces(&mut self) {
        if self
            .parser_config
            .allow_multiple_spaces_in_request_line_delimiters
        {
            self.read_buf.split_to(skip_spaces(self.read_buf.chunk()));
        }
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
        if b != b' ' {
            return offset;
        }
    }

    buf.len()
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

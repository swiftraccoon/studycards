//! A small HTTP/1.1 server for one local user.
//!
//! [`Server`] listens on the loopback interface only. It reads one request
//! per connection, passes it to a [`Handler`], writes the [`Response`], and
//! closes the connection. `GET`, `HEAD`, and `POST` with a form-encoded body
//! are supported.
//!
//! A web page from another site can make a browser send requests to a local
//! server. Three checks, applied before the handler runs, keep such requests
//! out:
//!
//! - The `Host` header must name this server: `127.0.0.1` or `localhost`,
//!   with the port. Any other request is answered with status 421.
//! - A request that the browser marks, in the `Sec-Fetch-Site` header, as
//!   coming from another site or from another port of this machine is
//!   answered with status 403 unless it opens a page in a tab or window.
//! - A `POST` must carry an `Origin` header that names this server. Any other
//!   `POST` is answered with status 403.
//!
//! The server has no login: a program on the same machine can connect and
//! send whatever a browser would, so the checks do not stand in for one.
//!
//! [`Server::run`] serves each connection on a thread of its own, at most 64
//! at a time; a connection beyond that number is answered with status 503.
//! The request line and headers of a request may take up to 16 KiB together,
//! and its body up to 1 MiB. A request has ten seconds to arrive and its
//! response ten seconds to be taken; a slower connection is closed. The
//! server has no shutdown procedure: it runs until accepting a connection
//! fails or the process ends.
//!
//! # Examples
//!
//! Reading a request and answering it. A [`Server`] does the same for each
//! connection, with the checks above between the two steps.
//!
//! ```
//! use studycards::http::{Handler, Method, Request, Response, Status};
//!
//! /// Answers with the path that was asked for.
//! struct Echo;
//!
//! impl Handler for Echo {
//!     fn handle(&self, request: &Request) -> Response {
//!         Response::text(Status::Ok, &request.segments().join("/"))
//!     }
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let bytes: &[u8] = b"GET /sets/demo HTTP/1.1\r\nHost: localhost\r\n\r\n";
//! let request = Request::read(bytes)?;
//! assert_eq!(request.method(), Method::Get);
//!
//! let response = Echo.handle(&request);
//! assert_eq!(response.status(), Status::Ok);
//! assert_eq!(response.body(), b"sets/demo");
//! # Ok(())
//! # }
//! ```
//!
//! The example in [`web`](crate::web) serves the application.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io::{self, ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

// The module documentation states the four limits below; keep it in step.

/// The most bytes accepted for the request line and headers together.
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// The most bytes accepted for a request body.
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// How long a request may take to arrive, and its response to be taken.
const IO_TIMEOUT: Duration = Duration::from_secs(10);
/// The most connections served at the same time.
const MAX_CONNECTIONS: usize = 64;
/// The bytes that end the request head.
const HEAD_END: &[u8] = b"\r\n\r\n";

/// A request method the server accepts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    /// The method `GET`.
    Get,
    /// The method `HEAD`.
    Head,
    /// The method `POST`.
    Post,
}

/// A response status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    /// Status 200.
    Ok,
    /// Status 303.
    SeeOther,
    /// Status 400.
    BadRequest,
    /// Status 403.
    Forbidden,
    /// Status 404.
    NotFound,
    /// Status 405.
    MethodNotAllowed,
    /// Status 413.
    PayloadTooLarge,
    /// Status 421.
    MisdirectedRequest,
    /// Status 431.
    HeaderFieldsTooLarge,
    /// Status 500.
    InternalServerError,
    /// Status 503.
    ServiceUnavailable,
}

impl Status {
    /// Returns the status code.
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::Ok => 200,
            Self::SeeOther => 303,
            Self::BadRequest => 400,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::PayloadTooLarge => 413,
            Self::MisdirectedRequest => 421,
            Self::HeaderFieldsTooLarge => 431,
            Self::InternalServerError => 500,
            Self::ServiceUnavailable => 503,
        }
    }

    /// Returns the reason phrase.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::SeeOther => "See Other",
            Self::BadRequest => "Bad Request",
            Self::Forbidden => "Forbidden",
            Self::NotFound => "Not Found",
            Self::MethodNotAllowed => "Method Not Allowed",
            Self::PayloadTooLarge => "Payload Too Large",
            Self::MisdirectedRequest => "Misdirected Request",
            Self::HeaderFieldsTooLarge => "Request Header Fields Too Large",
            Self::InternalServerError => "Internal Server Error",
            Self::ServiceUnavailable => "Service Unavailable",
        }
    }
}

/// Why a request could not be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestError {
    /// The connection failed or closed before a full request arrived.
    Disconnected(ErrorKind),
    /// The request does not follow the protocol; the status says how.
    Rejected(Status),
}

impl Display for RequestError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Disconnected(kind) => {
                write!(f, "the connection ended before a request arrived: {kind}")
            }
            Self::Rejected(status) => write!(
                f,
                "the request is answered with status {} {}",
                status.code(),
                status.reason()
            ),
        }
    }
}

impl Error for RequestError {}

impl From<io::Error> for RequestError {
    fn from(error: io::Error) -> Self {
        Self::Disconnected(error.kind())
    }
}

/// Name and value pairs from a query string or a form body.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Fields(Vec<(String, String)>);

impl Fields {
    /// Parses `application/x-www-form-urlencoded` text. Returns `None` when an
    /// escape is malformed or does not decode to UTF-8.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let pairs: Option<Vec<(String, String)>> = text
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                Some((
                    percent_decode(name, Plus::Space)?,
                    percent_decode(value, Plus::Space)?,
                ))
            })
            .collect();
        pairs.map(Self)
    }

    /// Returns the first value given for `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.all(name).next()
    }

    /// Iterates over every value given for `name`, in order.
    pub fn all<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a str> {
        self.0
            .iter()
            .filter(move |(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A request read from a connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    /// The request method.
    method: Method,
    /// The decoded, non-empty segments of the path.
    segments: Vec<String>,
    /// The fields of the query string.
    query: Fields,
    /// Header names in lowercase with their values.
    headers: Vec<(String, String)>,
    /// The request body.
    body: Vec<u8>,
}

impl Request {
    /// Reads one request from `stream`.
    ///
    /// The reader is taken by value. A caller that goes on using its reader
    /// passes `&mut reader`, which is a reader as well.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Disconnected`] when the stream fails or ends
    /// early, and [`RequestError::Rejected`] with the status to answer when
    /// the request is malformed, too large, or uses an unsupported feature.
    pub fn read(mut stream: impl Read) -> Result<Self, RequestError> {
        let (head, body_start) = read_head(&mut stream)?;
        let head = String::from_utf8(head)
            .map_err(|_not_utf8| RequestError::Rejected(Status::BadRequest))?;
        let mut request = Self::parse_head(&head).map_err(RequestError::Rejected)?;
        let length = request.content_length().map_err(RequestError::Rejected)?;
        if length > MAX_BODY_BYTES {
            return Err(RequestError::Rejected(Status::PayloadTooLarge));
        }
        let mut body = body_start;
        body.truncate(length);
        let missing = length.saturating_sub(body.len());
        let already = body.len();
        body.resize(length, 0);
        if missing > 0 {
            let tail = body.get_mut(already..).unwrap_or_default();
            stream.read_exact(tail)?;
        }
        request.body = body;
        Ok(request)
    }

    /// Parses the request line and headers.
    fn parse_head(head: &str) -> Result<Self, Status> {
        let mut lines = head.split("\r\n");
        let request_line = lines.next().ok_or(Status::BadRequest)?;
        let mut parts = request_line.split(' ');
        let (Some(method), Some(target), Some(version), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(Status::BadRequest);
        };
        if !matches!(version, "HTTP/1.1" | "HTTP/1.0") {
            return Err(Status::BadRequest);
        }
        let method = match method {
            "GET" => Method::Get,
            "HEAD" => Method::Head,
            "POST" => Method::Post,
            _ => return Err(Status::MethodNotAllowed),
        };
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if !path.starts_with('/') {
            return Err(Status::BadRequest);
        }
        let segments: Option<Vec<String>> = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(|segment| percent_decode(segment, Plus::Literal))
            .collect();
        let segments = segments.ok_or(Status::BadRequest)?;
        let query = Fields::parse(query).ok_or(Status::BadRequest)?;
        let headers: Option<Vec<(String, String)>> = lines
            .filter(|line| !line.is_empty())
            .map(|line| {
                let (name, value) = line.split_once(':')?;
                let valid_name = !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_graphic() && byte != b':');
                let value = value.trim_matches([' ', '\t']);
                valid_name.then(|| (name.to_ascii_lowercase(), value.to_owned()))
            })
            .collect();
        let headers = headers.ok_or(Status::BadRequest)?;
        // With two `Host` headers, which one names the server is anyone's
        // guess; the checks of `check_origin` rest on there being one.
        if headers.iter().filter(|(name, _)| name == "host").count() > 1 {
            return Err(Status::BadRequest);
        }
        Ok(Self {
            method,
            segments,
            query,
            headers,
            body: Vec::new(),
        })
    }

    /// Returns the declared body length.
    fn content_length(&self) -> Result<usize, Status> {
        if self.header("transfer-encoding").is_some() {
            return Err(Status::BadRequest);
        }
        let mut declared = self
            .headers
            .iter()
            .filter(|(name, _)| name == "content-length")
            .map(|(_, value)| {
                // `parse` alone would accept a sign.
                let digits = !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
                let length: Option<usize> = digits.then(|| value.parse().ok()).flatten();
                length.ok_or(Status::BadRequest)
            });
        let length = declared.next().transpose()?.unwrap_or(0);
        for other in declared {
            if other? != length {
                return Err(Status::BadRequest);
            }
        }
        Ok(length)
    }

    /// Returns the request method.
    #[must_use]
    pub const fn method(&self) -> Method {
        self.method
    }

    /// Returns the decoded segments of the path; `/sets/demo` gives
    /// `["sets", "demo"]` and `/` gives an empty list.
    #[must_use]
    pub fn segments(&self) -> Vec<&str> {
        self.segments.iter().map(String::as_str).collect()
    }

    /// Returns the fields of the query string.
    #[must_use]
    pub const fn query(&self) -> &Fields {
        &self.query
    }

    /// Returns the value of the first header called `name`, which is given in
    /// lowercase.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }

    /// Parses the body as a form. Returns `None` when the body is not valid
    /// form-encoded UTF-8.
    #[must_use]
    pub fn form(&self) -> Option<Fields> {
        Fields::parse(str::from_utf8(&self.body).ok()?)
    }

    /// Checks that the request was addressed to this server on `port` and
    /// that it was not sent by a page of another site.
    ///
    /// The server listens on `127.0.0.1` only, so the `Host` header must be
    /// `127.0.0.1` or `localhost` with `port`. A browser states where a
    /// request comes from in the `Sec-Fetch-Site` header: a request from
    /// another site, or from another port of this machine, passes only when
    /// it opens a page in a tab or window. A `POST` must also carry an
    /// `Origin` header that names this server. A client that is not a browser
    /// can send any header it likes; the checks do not stand in for a login.
    ///
    /// A request that does not pass is refused with the status returned:
    /// [`Status::MisdirectedRequest`] when the `Host` header does not name
    /// this server, and [`Status::Forbidden`] when the request comes from
    /// elsewhere or a `POST` lacks an `Origin` header naming this server.
    ///
    /// The module documentation states these checks for the users of a
    /// [`Server`]; keep it in step.
    fn check_origin(&self, port: u16) -> Result<(), Status> {
        let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        let host = self.header("host").ok_or(Status::MisdirectedRequest)?;
        if !hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
        {
            return Err(Status::MisdirectedRequest);
        }
        if let Some(site) = self.header("sec-fetch-site") {
            let from_here = matches!(site, "same-origin" | "none");
            let opens_a_page = self.method != Method::Post
                && self.header("sec-fetch-mode") == Some("navigate")
                && self.header("sec-fetch-dest") == Some("document");
            if !from_here && !opens_a_page {
                return Err(Status::Forbidden);
            }
        }
        if self.method == Method::Post {
            let origin = self.header("origin").ok_or(Status::Forbidden)?;
            let from_here = origin.strip_prefix("http://").is_some_and(|source| {
                hosts
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(source))
            });
            if !from_here {
                return Err(Status::Forbidden);
            }
        }
        Ok(())
    }
}

/// Reads up to the end of the request head. Returns the head without its
/// terminator and any body bytes that arrived with it.
fn read_head(stream: &mut impl Read) -> Result<(Vec<u8>, Vec<u8>), RequestError> {
    let too_large = RequestError::Rejected(Status::HeaderFieldsTooLarge);
    let mut received = Vec::new();
    let mut chunk = [0_u8; 2048];
    // No terminator begins before this position, so each byte is searched a
    // fixed number of times however the request is cut up on its way.
    let mut searched = 0_usize;
    loop {
        let found = received.get(searched..).and_then(|fresh| {
            fresh
                .windows(HEAD_END.len())
                .position(|window| window == HEAD_END)
        });
        if let Some(offset) = found {
            let end = searched.saturating_add(offset);
            if end > MAX_HEAD_BYTES {
                return Err(too_large);
            }
            let body_start = received.split_off(end.saturating_add(HEAD_END.len()));
            received.truncate(end);
            return Ok((received, body_start));
        }
        if received.len() > MAX_HEAD_BYTES.saturating_add(HEAD_END.len()) {
            return Err(too_large);
        }
        // A terminator can begin in the last bytes received so far.
        searched = received
            .len()
            .saturating_sub(HEAD_END.len().saturating_sub(1));
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err(RequestError::Disconnected(ErrorKind::UnexpectedEof));
        }
        received.extend(chunk.iter().take(count));
    }
}

/// What a `+` stands for in percent-encoded text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Plus {
    /// A space, as in a query string or a form body.
    Space,
    /// A plus sign, as in a path.
    Literal,
}

/// Decodes `%XX` escapes, and `+` as `plus` says. Returns `None` for a
/// malformed escape, a NUL byte, or bytes that are not UTF-8.
fn percent_decode(text: &str, plus: Plus) -> Option<String> {
    let mut decoded = Vec::with_capacity(text.len());
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        let value = match byte {
            b'%' => {
                let high = char::from(bytes.next()?).to_digit(16)?;
                let low = char::from(bytes.next()?).to_digit(16)?;
                u8::try_from(high.checked_mul(16)?.checked_add(low)?).ok()?
            }
            b'+' if plus == Plus::Space => b' ',
            other => other,
        };
        if value == 0 {
            return None;
        }
        decoded.push(value);
    }
    String::from_utf8(decoded).ok()
}

/// A response to write to a connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    /// The status line.
    status: Status,
    /// Headers besides the ones every response carries.
    headers: Vec<(&'static str, String)>,
    /// The response body.
    body: Vec<u8>,
}

impl Response {
    /// Creates a response with a body of the given media type.
    #[must_use]
    pub fn new(status: Status, content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: vec![("Content-Type", content_type.to_owned())],
            body,
        }
    }

    /// Creates an HTML page.
    #[must_use]
    pub fn html(status: Status, page: String) -> Self {
        Self::new(status, "text/html; charset=utf-8", page.into_bytes())
    }

    /// Creates a plain-text response.
    #[must_use]
    pub fn text(status: Status, text: &str) -> Self {
        Self::new(
            status,
            "text/plain; charset=utf-8",
            text.as_bytes().to_vec(),
        )
    }

    /// Creates a redirect that makes the browser `GET` the path `location`.
    #[must_use]
    pub fn see_other(location: &str) -> Self {
        Self {
            status: Status::SeeOther,
            headers: vec![("Location", location.to_owned())],
            body: Vec::new(),
        }
    }

    /// Returns the status.
    #[must_use]
    pub const fn status(&self) -> Status {
        self.status
    }

    /// Returns the value of the header called `name`.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Returns the body.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Writes as much of the response as `extent` says.
    fn write(&self, stream: &mut impl Write, extent: Extent) -> io::Result<()> {
        let mut head = format!(
            "HTTP/1.1 {} {}\r\n",
            self.status.code(),
            self.status.reason()
        );
        let fixed = [
            ("Content-Length", self.body.len().to_string()),
            ("Connection", "close".to_owned()),
            ("Cache-Control", "no-store".to_owned()),
            ("X-Content-Type-Options", "nosniff".to_owned()),
            // `no-referrer` would make browsers send `Origin: null` with form
            // posts, which `Request::check_origin` refuses.
            ("Referrer-Policy", "same-origin".to_owned()),
            (
                "Content-Security-Policy",
                "default-src 'none'; style-src 'self'; script-src 'self'; img-src 'self'; \
                 form-action 'self'; base-uri 'none'; frame-ancestors 'none'"
                    .to_owned(),
            ),
            // The next two keep the pages of other sites from embedding a
            // response and from holding on to a window that shows one.
            ("Cross-Origin-Resource-Policy", "same-origin".to_owned()),
            ("Cross-Origin-Opener-Policy", "same-origin".to_owned()),
            // For browsers that do not know `frame-ancestors`.
            ("X-Frame-Options", "DENY".to_owned()),
        ];
        for (name, value) in self.headers.iter().chain(&fixed) {
            // Header values come from this program; line breaks are removed
            // so that no value can start a new header.
            let value: String = value
                .chars()
                .filter(|character| !character.is_control())
                .collect();
            head.push_str(name);
            head.push_str(": ");
            head.push_str(&value);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes())?;
        if extent == Extent::Whole {
            stream.write_all(&self.body)?;
        }
        stream.flush()
    }
}

/// How much of a response is written to a connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Extent {
    /// The status line, the headers, and the body.
    Whole,
    /// The status line and the headers, which is the answer to `HEAD`.
    HeadOnly,
}

/// Produces the response to a request.
pub trait Handler: Send + Sync + 'static {
    /// Handles a request that passed the checks described in the [module
    /// documentation](self).
    fn handle(&self, request: &Request) -> Response;
}

/// A listening server.
#[derive(Debug)]
pub struct Server {
    /// The listening socket.
    listener: TcpListener,
    /// The port the socket is bound to.
    port: u16,
    /// How long a request may take to arrive, and its response to be taken.
    timeout: Duration,
    /// The most connections served at the same time.
    max_connections: usize,
}

impl Server {
    /// Starts listening on the loopback interface. Port 0 picks a free port.
    ///
    /// # Errors
    ///
    /// Returns the error from binding the socket, for example when the port
    /// is in use.
    pub fn bind(port: u16) -> io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
        let bound = listener.local_addr()?.port();
        Ok(Self {
            listener,
            port: bound,
            timeout: IO_TIMEOUT,
            max_connections: MAX_CONNECTIONS,
        })
    }

    /// Returns the port the server listens on.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Serves connections until accepting one fails, each on its own thread.
    /// The threads share `handler`.
    ///
    /// # Errors
    ///
    /// Returns the error that stopped the server from accepting connections.
    pub fn run(&self, handler: impl Handler) -> io::Result<()> {
        let handler = Arc::new(handler);
        let active = Arc::new(AtomicUsize::new(0));
        for stream in self.listener.incoming() {
            let stream = match stream {
                Ok(stream) => stream,
                Err(error) if is_transient(&error) => continue,
                Err(error) => return Err(error),
            };
            let slot = ConnectionSlot::claim(Arc::clone(&active));
            if slot.position > self.max_connections {
                refuse(stream);
                continue;
            }
            let handler = Arc::clone(&handler);
            let (port, timeout) = (self.port, self.timeout);
            // When the thread cannot start, the closure is dropped, which
            // closes the connection and releases the slot.
            drop(thread::Builder::new().spawn(move || {
                serve(&stream, handler.as_ref(), port, timeout);
                drop(slot);
            }));
        }
        Ok(())
    }
}

/// One direction of a connection, with a limit on the time that all of its
/// reads, or all of its writes, may take together.
///
/// The limits that a socket offers apply to each read or write. A peer that
/// sent or took a byte every few seconds would hold its connection, and one
/// of the places for connections, for hours.
struct Timed<'connection> {
    /// The connection.
    stream: &'connection TcpStream,
    /// When the reading or writing began.
    started: Instant,
    /// The time allowed in all.
    allowed: Duration,
}

impl<'connection> Timed<'connection> {
    /// Starts the clock for reading from `stream` or for writing to it.
    fn new(stream: &'connection TcpStream, allowed: Duration) -> Self {
        Self {
            stream,
            started: Instant::now(),
            allowed,
        }
    }

    /// Returns the time that is left, or an error when none is.
    fn remaining(&self) -> io::Result<Duration> {
        let left = self.allowed.saturating_sub(self.started.elapsed());
        if left.is_zero() {
            Err(ErrorKind::TimedOut.into())
        } else {
            Ok(left)
        }
    }
}

impl Read for Timed<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        let mut stream = self.stream;
        stream.read(buf)
    }
}

impl Write for Timed<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        let mut stream = self.stream;
        stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut stream = self.stream;
        stream.flush()
    }
}

/// One of the connections being served; dropping it frees the place.
struct ConnectionSlot {
    /// The number of connections being served.
    active: Arc<AtomicUsize>,
    /// This connection's place among them, counted from one.
    position: usize,
}

impl ConnectionSlot {
    /// Counts one more connection in `active`.
    fn claim(active: Arc<AtomicUsize>) -> Self {
        let position = active.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        Self { active, position }
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Returns `true` for accept errors that concern one connection only.
fn is_transient(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset | ErrorKind::Interrupted
    )
}

/// Tells a client that the server is busy.
fn refuse(mut stream: TcpStream) {
    // The client may already be gone; there is nobody else to tell.
    drop(stream.set_write_timeout(Some(IO_TIMEOUT)));
    drop(
        Response::text(Status::ServiceUnavailable, "Too many connections.")
            .write(&mut stream, Extent::Whole),
    );
}

/// Reads a request from `stream`, answers it, and ends the exchange. The
/// request has `timeout` to arrive, and the response has `timeout` to be
/// taken.
fn serve(stream: &TcpStream, handler: &dyn Handler, port: u16, timeout: Duration) {
    let (response, extent) = match Request::read(Timed::new(stream, timeout)) {
        Ok(request) => {
            let extent = if request.method() == Method::Head {
                Extent::HeadOnly
            } else {
                Extent::Whole
            };
            (respond(&request, handler, port), extent)
        }
        Err(RequestError::Rejected(status)) => {
            (Response::text(status, status.reason()), Extent::Whole)
        }
        Err(RequestError::Disconnected(_)) => return,
    };
    // A client that went away, or takes too long, cannot be answered; the
    // failure concerns nobody else.
    drop(response.write(&mut Timed::new(stream, timeout), extent));
    drop(stream.shutdown(Shutdown::Write));
}

/// Applies the origin checks and runs the handler.
fn respond(request: &Request, handler: &dyn Handler, port: u16) -> Response {
    if let Err(status) = request.check_origin(port) {
        return Response::text(status, status.reason());
    }
    // Defence in depth: the lints of this crate keep panics out of shipped
    // code, and no behaviour depends on one being caught. If a handler panics
    // all the same, the client is told instead of losing its connection.
    panic::catch_unwind(AssertUnwindSafe(|| handler.handle(request))).unwrap_or_else(|_| {
        let status = Status::InternalServerError;
        Response::text(status, status.reason())
    })
}

#[cfg(test)]
mod tests {
    use std::io::{self, ErrorKind, Read};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
    use std::thread;
    use std::time::Duration;

    use super::{Extent, Fields, Handler, Method, Plus, Request, RequestError, Response};
    use super::{MAX_BODY_BYTES, MAX_HEAD_BYTES};
    use super::{Server, Status, Timed, percent_decode, serve};

    /// Yields its content a few bytes at a time.
    struct Trickle {
        content: Vec<u8>,
        step: usize,
    }

    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let count = self.content.len().min(buf.len()).min(self.step);
            for (target, byte) in buf.iter_mut().zip(self.content.drain(..count)) {
                *target = byte;
            }
            Ok(count)
        }
    }

    fn read(text: &str) -> Result<Request, RequestError> {
        Request::read(text.as_bytes())
    }

    fn rejected(text: &str) -> Status {
        match read(text) {
            Err(RequestError::Rejected(status)) => status,
            Err(RequestError::Disconnected(kind)) => panic!("the connection ended: {kind}"),
            Ok(request) => panic!("a {:?} request was accepted", request.method()),
        }
    }

    #[test]
    fn reads_a_get_request() {
        let request = read(
            "GET /sets/solar%20system/study?topic=a+b&x=%C3%A9 HTTP/1.1\r\n\
             Host: localhost:80\r\nX-Thing: \t padded \t\r\n\r\n",
        )
        .expect("a valid request");
        assert_eq!(request.method(), Method::Get, "method");
        assert_eq!(
            request.segments(),
            ["sets", "solar system", "study"],
            "decoded path segments"
        );
        assert_eq!(
            request.query().get("topic"),
            Some("a b"),
            "plus decodes to a space in a query"
        );
        assert_eq!(request.query().get("x"), Some("é"), "UTF-8 escapes decode");
        assert_eq!(request.query().get("missing"), None, "absent field");
        assert_eq!(
            request.header("host"),
            Some("localhost:80"),
            "header lookup is by lowercase name"
        );
        assert_eq!(
            request.header("x-thing"),
            Some("padded"),
            "header values are trimmed"
        );
        assert_eq!(
            request.form(),
            Some(Fields::default()),
            "an empty body is an empty form"
        );
    }

    #[test]
    fn the_root_path_has_no_segments() {
        let request = read("GET / HTTP/1.1\r\n\r\n").expect("a valid request");
        assert!(request.segments().is_empty(), "no segments");
        let doubled = read("GET //a///b/ HTTP/1.0\r\n\r\n").expect("a valid request");
        assert_eq!(doubled.segments(), ["a", "b"], "empty segments are ignored");
    }

    #[test]
    fn reads_a_form_post_split_across_reads() {
        let body = "card=q-1&choice=2&note=a%26b+c&wrong=x&wrong=y";
        let text = format!(
            "POST /answer HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\n\
             Content-Length: {}\r\n\r\n{body}extra bytes",
            body.len()
        );
        let stream = Trickle {
            content: text.into_bytes(),
            step: 7,
        };
        let request = Request::read(stream).expect("a valid request");
        assert_eq!(request.method(), Method::Post, "method");
        let form = request.form().expect("a valid form");
        assert_eq!(form.get("card"), Some("q-1"), "plain value");
        assert_eq!(
            form.get("note"),
            Some("a&b c"),
            "escapes and plus decode in a form"
        );
        let wrong: Vec<&str> = form.all("wrong").collect();
        assert_eq!(wrong, ["x", "y"], "repeated fields keep their order");
        assert_eq!(
            form.get("extra"),
            None,
            "bytes after the declared length are ignored"
        );
    }

    #[test]
    fn rejects_malformed_requests() {
        let cases = [
            ("GARBAGE\r\n\r\n", Status::BadRequest),
            ("GET /\r\n\r\n", Status::BadRequest),
            ("GET / HTTP/2\r\n\r\n", Status::BadRequest),
            ("GET  / HTTP/1.1\r\n\r\n", Status::BadRequest),
            ("GET relative HTTP/1.1\r\n\r\n", Status::BadRequest),
            (
                "GET http://example.com/ HTTP/1.1\r\n\r\n",
                Status::BadRequest,
            ),
            ("GET /%zz HTTP/1.1\r\n\r\n", Status::BadRequest),
            ("GET /a%00b HTTP/1.1\r\n\r\n", Status::BadRequest),
            ("GET /%ff HTTP/1.1\r\n\r\n", Status::BadRequest),
            ("GET /?x=%4 HTTP/1.1\r\n\r\n", Status::BadRequest),
            (
                "GET / HTTP/1.1\r\nNo colon here\r\n\r\n",
                Status::BadRequest,
            ),
            ("GET / HTTP/1.1\r\nBad Name: x\r\n\r\n", Status::BadRequest),
            ("DELETE / HTTP/1.1\r\n\r\n", Status::MethodNotAllowed),
            (
                "POST / HTTP/1.1\r\nContent-Length: many\r\n\r\n",
                Status::BadRequest,
            ),
            (
                "POST / HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nab",
                Status::BadRequest,
            ),
            (
                "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
                Status::BadRequest,
            ),
            (
                "POST / HTTP/1.1\r\nContent-Length: +1\r\n\r\na",
                Status::BadRequest,
            ),
            (
                "POST / HTTP/1.1\r\nContent-Length:\r\n\r\n",
                Status::BadRequest,
            ),
            (
                "GET / HTTP/1.1\r\nHost: localhost:80\r\nHost: evil.example\r\n\r\n",
                Status::BadRequest,
            ),
            (
                "GET / HTTP/1.1\r\nHost: localhost:80\r\nhost: localhost:80\r\n\r\n",
                Status::BadRequest,
            ),
        ];
        for (text, status) in cases {
            assert_eq!(rejected(text), status, "status for {text:?}");
        }
    }

    #[test]
    fn the_end_of_the_head_is_found_however_the_request_is_cut_up() {
        let text = "POST /a HTTP/1.1\r\nX: a\rb\r\rc\r\nContent-Length: 6\r\n\r\n\r\n\r\nab";
        for step in 1..=text.len() {
            let stream = Trickle {
                content: text.as_bytes().to_vec(),
                step,
            };
            let request = Request::read(stream).expect("a valid request");
            assert_eq!(
                request.header("x"),
                Some("a\rb\r\rc"),
                "the head ends at its first blank line, {step} bytes at a time"
            );
            assert_eq!(
                request.body, b"\r\n\r\nab",
                "the body follows it, {step} bytes at a time"
            );
        }
    }

    #[test]
    fn rejects_oversized_requests() {
        // "GET / HTTP/1.1\r\nX: " is 19 bytes; the value pads the head to a
        // chosen size.
        let head_of =
            |size: usize| format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(size - 19));
        assert!(
            read(&head_of(MAX_HEAD_BYTES)).is_ok(),
            "a head of exactly the limit is accepted"
        );
        assert_eq!(
            rejected(&head_of(MAX_HEAD_BYTES + 1)),
            Status::HeaderFieldsTooLarge,
            "a head one byte over the limit"
        );
        let endless = format!("GET / HTTP/1.1\r\nX: {}", "a".repeat(MAX_HEAD_BYTES * 2));
        assert_eq!(
            rejected(&endless),
            Status::HeaderFieldsTooLarge,
            "a head that never ends"
        );

        let body_of = |size: usize| format!("POST / HTTP/1.1\r\nContent-Length: {size}\r\n\r\n");
        assert_eq!(
            rejected(&body_of(MAX_BODY_BYTES + 1)),
            Status::PayloadTooLarge,
            "an oversized body"
        );
        let largest = format!("{}{}", body_of(MAX_BODY_BYTES), "a".repeat(MAX_BODY_BYTES));
        assert!(
            read(&largest).is_ok(),
            "a body of exactly the limit is accepted"
        );
    }

    #[test]
    fn a_connection_that_ends_early_is_reported_as_disconnected() {
        for text in [
            "",
            "GET / HTTP/1.1\r\nHost: x",
            "POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\nab",
        ] {
            assert!(
                matches!(read(text), Err(RequestError::Disconnected(_))),
                "{text:?} ends early"
            );
        }
    }

    #[test]
    fn a_repeated_content_length_is_accepted_when_the_values_agree() {
        let request = read("POST / HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\nab")
            .expect("a valid request");
        assert_eq!(request.body, b"ab", "the body has the declared length");
    }

    #[test]
    fn request_errors_say_what_happened() {
        assert_eq!(
            RequestError::Rejected(Status::PayloadTooLarge).to_string(),
            "the request is answered with status 413 Payload Too Large",
            "message for a rejected request"
        );
        assert_eq!(
            RequestError::Disconnected(ErrorKind::UnexpectedEof).to_string(),
            "the connection ended before a request arrived: unexpected end of file",
            "message for a connection that ended early"
        );
        let failed = io::Error::new(ErrorKind::ConnectionReset, "reset");
        assert_eq!(
            RequestError::from(failed),
            RequestError::Disconnected(ErrorKind::ConnectionReset),
            "an I/O error is kept by its kind"
        );
    }

    #[test]
    fn a_body_that_is_not_a_form_has_no_fields() {
        let request =
            read("POST / HTTP/1.1\r\nContent-Length: 3\r\n\r\na=%").expect("a valid request");
        assert_eq!(request.form(), None, "a malformed escape is rejected");
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(
            percent_decode("a%20b+c", Plus::Literal).as_deref(),
            Some("a b+c"),
            "plus is literal in a path"
        );
        assert_eq!(
            percent_decode("a%20b+c", Plus::Space).as_deref(),
            Some("a b c"),
            "plus is a space in a form"
        );
        assert_eq!(
            percent_decode("%e6%97%a5", Plus::Space).as_deref(),
            Some("日"),
            "lowercase hex and UTF-8"
        );
        assert_eq!(
            percent_decode("%2F..%2f", Plus::Literal).as_deref(),
            Some("/../"),
            "escaped slashes decode"
        );
        for bad in ["%", "%1", "%g1", "%00", "%c3", "%+1"] {
            assert_eq!(
                percent_decode(bad, Plus::Space),
                None,
                "{bad:?} is rejected"
            );
        }
    }

    #[test]
    fn origin_checks() {
        let request = |method: &str, headers: &str| {
            read(&format!("{method} / HTTP/1.1\r\n{headers}\r\n")).expect("a valid request")
        };
        let ok: Result<(), Status> = Ok(());
        for host in ["127.0.0.1:8000", "localhost:8000", "LOCALHOST:8000"] {
            assert_eq!(
                request("GET", &format!("Host: {host}\r\n")).check_origin(8000),
                ok,
                "host {host}"
            );
        }
        for host in [
            "127.0.0.1:9000",
            "localhost",
            "evil.example:8000",
            "127.0.0.1.evil.example:8000",
            "localhost.:8000",
            "[::1]:8000",
            "\u{a0}localhost:8000",
        ] {
            assert_eq!(
                request("GET", &format!("Host: {host}\r\n")).check_origin(8000),
                Err(Status::MisdirectedRequest),
                "host {host}"
            );
        }
        assert_eq!(
            request("GET", "").check_origin(8000),
            Err(Status::MisdirectedRequest),
            "no host"
        );

        let post = |origin: &str| {
            request(
                "POST",
                &format!("Host: localhost:8000\r\n{origin}Content-Length: 0\r\n"),
            )
            .check_origin(8000)
        };
        assert_eq!(post("Origin: http://localhost:8000\r\n"), ok, "same origin");
        assert_eq!(
            post("Origin: http://127.0.0.1:8000\r\n"),
            ok,
            "another name of this server"
        );
        assert_eq!(post(""), Err(Status::Forbidden), "no origin");
        for origin in [
            "https://localhost:8000",
            "http://evil.example",
            "http://localhost:9000",
            "http://[::1]:8000",
            "null",
        ] {
            assert_eq!(
                post(&format!("Origin: {origin}\r\n")),
                Err(Status::Forbidden),
                "origin {origin}"
            );
        }
    }

    #[test]
    fn requests_from_other_sites_are_refused_unless_they_open_a_page() {
        let check = |method: &str, site: &str, mode: &str, destination: &str| {
            let origin = if method == "POST" {
                "Origin: http://localhost:8000\r\nContent-Length: 0\r\n"
            } else {
                ""
            };
            read(&format!(
                "{method} / HTTP/1.1\r\nHost: localhost:8000\r\n{origin}Sec-Fetch-Site: {site}\r\n\
                 Sec-Fetch-Mode: {mode}\r\nSec-Fetch-Dest: {destination}\r\n\r\n"
            ))
            .expect("a valid request")
            .check_origin(8000)
        };
        let allowed = [
            ("GET", "same-origin", "no-cors", "style"),
            ("GET", "same-origin", "navigate", "document"),
            ("POST", "same-origin", "navigate", "document"),
            ("GET", "none", "navigate", "document"),
            ("GET", "cross-site", "navigate", "document"),
            ("HEAD", "same-site", "navigate", "document"),
        ];
        for (method, site, mode, destination) in allowed {
            assert_eq!(
                check(method, site, mode, destination),
                Ok(()),
                "{method} from {site} as {mode} for {destination}"
            );
        }
        let refused = [
            ("GET", "cross-site", "no-cors", "image"),
            ("GET", "cross-site", "no-cors", "script"),
            ("GET", "cross-site", "cors", "empty"),
            ("GET", "cross-site", "navigate", "iframe"),
            ("GET", "same-site", "no-cors", "image"),
            ("HEAD", "cross-site", "no-cors", "empty"),
            ("POST", "cross-site", "navigate", "document"),
            ("POST", "same-site", "navigate", "document"),
            ("GET", "Same-Origin", "no-cors", "image"),
        ];
        for (method, site, mode, destination) in refused {
            assert_eq!(
                check(method, site, mode, destination),
                Err(Status::Forbidden),
                "{method} from {site} as {mode} for {destination}"
            );
        }
    }

    #[test]
    fn responses_carry_length_and_security_headers() {
        let mut written = Vec::new();
        Response::html(Status::Ok, "<p>hi</p>".to_owned())
            .write(&mut written, Extent::Whole)
            .expect("written");
        let text = String::from_utf8(written).expect("ASCII response");
        assert!(
            text.starts_with("HTTP/1.1 200 OK\r\n"),
            "status line: {text}"
        );
        for header in [
            "Content-Type: text/html; charset=utf-8\r\n",
            "Content-Length: 9\r\n",
            "Connection: close\r\n",
            "Cache-Control: no-store\r\n",
            "X-Content-Type-Options: nosniff\r\n",
            "Referrer-Policy: same-origin\r\n",
            "Content-Security-Policy: default-src 'none';",
            "frame-ancestors 'none'\r\n",
            "Cross-Origin-Resource-Policy: same-origin\r\n",
            "Cross-Origin-Opener-Policy: same-origin\r\n",
            "X-Frame-Options: DENY\r\n",
        ] {
            assert!(text.contains(header), "{header:?} is present in:\n{text}");
        }
        assert!(
            text.ends_with("\r\n\r\n<p>hi</p>"),
            "the body follows the head: {text}"
        );

        let mut head_only = Vec::new();
        Response::text(Status::NotFound, "missing")
            .write(&mut head_only, Extent::HeadOnly)
            .expect("written");
        let head = String::from_utf8(head_only).expect("ASCII response");
        assert!(
            head.starts_with("HTTP/1.1 404 Not Found\r\n"),
            "status line: {head}"
        );
        assert!(
            head.contains("Content-Length: 7\r\n"),
            "the length of the omitted body is declared"
        );
        assert!(head.ends_with("\r\n\r\n"), "no body is written: {head}");
    }

    #[test]
    fn redirects_cannot_smuggle_headers() {
        let response = Response::see_other("/sets/a\r\nSet-Cookie: x=1");
        assert_eq!(response.status(), Status::SeeOther, "status");
        let mut written = Vec::new();
        response
            .write(&mut written, Extent::Whole)
            .expect("written");
        let text = String::from_utf8(written).expect("ASCII response");
        assert!(
            text.contains("Location: /sets/aSet-Cookie: x=1\r\n"),
            "line breaks are removed: {text}"
        );
        assert_eq!(
            text.matches("\r\n").count(),
            12,
            "no extra header line appears: {text}"
        );
    }

    #[test]
    fn every_status_has_a_code_and_reason() {
        let statuses = [
            (Status::Ok, 200),
            (Status::SeeOther, 303),
            (Status::BadRequest, 400),
            (Status::Forbidden, 403),
            (Status::NotFound, 404),
            (Status::MethodNotAllowed, 405),
            (Status::PayloadTooLarge, 413),
            (Status::MisdirectedRequest, 421),
            (Status::HeaderFieldsTooLarge, 431),
            (Status::InternalServerError, 500),
            (Status::ServiceUnavailable, 503),
        ];
        for (status, code) in statuses {
            assert_eq!(status.code(), code, "code of {status:?}");
            assert!(!status.reason().is_empty(), "reason of {status:?}");
        }
    }

    /// Echoes the method and path, and panics on `/panic`.
    struct Echo;

    impl Handler for Echo {
        fn handle(&self, request: &Request) -> Response {
            assert_ne!(request.segments(), ["panic"], "requested panic");
            let note = request
                .form()
                .and_then(|form| form.get("note").map(str::to_owned))
                .unwrap_or_default();
            Response::text(
                Status::Ok,
                &format!(
                    "{:?} /{} {note}",
                    request.method(),
                    request.segments().join("/")
                ),
            )
        }
    }

    /// Accepts one connection and serves it on the calling thread.
    fn serve_next(server: &Server, handler: &dyn Handler) {
        let (stream, _) = server.listener.accept().expect("a connection arrives");
        serve(&stream, handler, server.port, server.timeout);
    }

    /// Sends the request that `request` writes for the port of a fresh server
    /// and returns the raw response.
    fn exchange(request: impl FnOnce(u16) -> String) -> String {
        use std::io::Write as _;

        let server = Server::bind(0).expect("a free port");
        let port = server.port();
        let request = request(port);
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
            stream.write_all(request.as_bytes()).expect("request sent");
            let mut response = String::new();
            stream.read_to_string(&mut response).expect("response read");
            response
        });
        serve_next(&server, &Echo);
        client.join().expect("the client finished")
    }

    #[test]
    fn serves_requests_over_tcp() {
        let got = exchange(|port| format!("GET /a/b HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n"));
        assert!(got.starts_with("HTTP/1.1 200 OK\r\n"), "status: {got}");
        assert!(
            got.ends_with("Get /a/b "),
            "the handler saw the request: {got}"
        );

        let posted = exchange(|port| {
            format!(
                "POST /save HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
                 Origin: http://127.0.0.1:{port}\r\nContent-Length: 7\r\n\r\nnote=hi"
            )
        });
        assert!(
            posted.ends_with("Post /save hi"),
            "the form reached the handler: {posted}"
        );

        let head = exchange(|port| format!("HEAD /a HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n"));
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "status: {head}");
        assert!(
            head.ends_with("\r\n\r\n"),
            "a HEAD response has no body: {head}"
        );
    }

    #[test]
    fn refuses_requests_from_elsewhere_before_the_handler_runs() {
        let foreign_host =
            exchange(|_| "GET /panic HTTP/1.1\r\nHost: evil.example\r\n\r\n".to_owned());
        assert!(
            foreign_host.starts_with("HTTP/1.1 421 "),
            "a foreign host is refused: {foreign_host}"
        );
        let foreign_origin = exchange(|port| {
            format!(
                "POST /panic HTTP/1.1\r\nHost: localhost:{port}\r\n\
                 Origin: http://evil.example\r\nContent-Length: 0\r\n\r\n"
            )
        });
        assert!(
            foreign_origin.starts_with("HTTP/1.1 403 "),
            "a foreign origin is refused: {foreign_origin}"
        );
        let unknown_method =
            exchange(|port| format!("BREW /panic HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n"));
        assert!(
            unknown_method.starts_with("HTTP/1.1 405 "),
            "an unknown method is refused: {unknown_method}"
        );
    }

    #[test]
    fn a_panicking_handler_yields_a_server_error() {
        let response =
            exchange(|port| format!("GET /panic HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n"));
        assert!(
            response.starts_with("HTTP/1.1 500 "),
            "the panic becomes a 500: {response}"
        );
    }

    #[test]
    fn a_request_that_arrives_too_slowly_gets_no_answer() {
        use std::io::Write as _;

        let mut server = Server::bind(0).expect("a free port");
        server.timeout = Duration::from_millis(200);
        let port = server.port();
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
            let request = format!("GET /slow HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n");
            // No pause is as long as the limit; together they are far longer.
            for piece in request.as_bytes().chunks(4) {
                if stream.write_all(piece).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            let mut response = String::new();
            // The server has closed the connection, which can end the read
            // with an error instead of an empty answer.
            drop(stream.read_to_string(&mut response));
            response
        });
        serve_next(&server, &Echo);
        assert_eq!(
            client.join().expect("the client finished"),
            "",
            "a request that runs out of time is not answered"
        );
    }

    #[test]
    fn a_response_that_is_taken_too_slowly_is_abandoned() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a free port");
        let address = listener.local_addr().expect("the socket has an address");
        let (finished, writing): (Sender<()>, Receiver<()>) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).expect("connects");
            let mut buffer = [0_u8; 1024];
            // Takes a little at a time for as long as the writer goes on.
            while matches!(writing.try_recv(), Err(TryRecvError::Empty)) {
                drop(stream.read(&mut buffer));
                thread::sleep(Duration::from_millis(5));
            }
        });
        let (served, _) = listener.accept().expect("a connection arrives");
        // The body is larger than what the buffers of a connection hold.
        let response = Response::new(
            Status::Ok,
            "application/octet-stream",
            vec![0_u8; 64 * 1024 * 1024],
        );
        let outcome = response.write(
            &mut Timed::new(&served, Duration::from_millis(200)),
            Extent::Whole,
        );
        drop(finished);
        reader.join().expect("the reader finished");
        assert!(
            outcome.is_err(),
            "writing stops when the time is up, although the reader keeps taking bytes"
        );
    }

    #[test]
    fn a_connection_beyond_the_limit_is_told_that_the_server_is_busy() {
        let mut server = Server::bind(0).expect("a free port");
        server.max_connections = 2;
        let port = server.port();
        // The server thread runs until the test process exits.
        drop(thread::spawn(move || server.run(Echo)));
        let connect = || TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");

        // A connection that sends nothing keeps its place while the server
        // waits for its request.
        let waiting = [connect(), connect()];
        let mut refused = connect();
        let mut response = String::new();
        refused
            .read_to_string(&mut response)
            .expect("response read");
        assert!(
            response.starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
            "the third connection is refused: {response}"
        );
        assert!(
            response.ends_with("\r\n\r\nToo many connections."),
            "the response says why: {response}"
        );
        drop(waiting);
    }

    #[test]
    fn run_serves_connections_on_threads() {
        use std::io::Write as _;

        let server = Server::bind(0).expect("a free port");
        let port = server.port();
        // The server thread runs until the test process exits.
        drop(thread::spawn(move || server.run(Echo)));
        for path in ["one", "two", "three"] {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
            write!(
                stream,
                "GET /{path} HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n"
            )
            .expect("request sent");
            let mut response = String::new();
            stream.read_to_string(&mut response).expect("response read");
            assert!(
                response.ends_with(&format!("Get /{path} ")),
                "response for {path}: {response}"
            );
        }
    }
}

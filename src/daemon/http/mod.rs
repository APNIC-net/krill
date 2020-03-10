use std::str::FromStr;
use std::sync::{RwLockReadGuard, RwLockWriteGuard};
use std::{fmt, io};

use serde::de::DeserializeOwned;
use serde::Serialize;

use bytes::{Buf, BufMut, Bytes};

use hyper::body::HttpBody;
use hyper::http::uri::PathAndQuery;
use hyper::{Body, Method, StatusCode};

use crate::commons::api::Token;
use crate::commons::error::Error;
use crate::commons::remote::{rfc6492, rfc8181};
use crate::daemon::auth::Auth;
use crate::daemon::http::server::State;
use crate::daemon::krillserver::KrillServer;

pub mod server;
pub mod statics;
pub mod tls;
pub mod tls_keys;

//----------- ContentType ----------------------------------------------------

enum ContentType {
    Cert,
    Json,
    Html,
    Rfc8181,
    Rfc6492,
    Text,
    Xml,
}

impl AsRef<str> for ContentType {
    fn as_ref(&self) -> &str {
        match self {
            ContentType::Cert => "application/x-x509-ca-cert",
            ContentType::Json => "application/json",
            ContentType::Html => "text/html;charset=utf-8",
            ContentType::Rfc8181 => rfc8181::CONTENT_TYPE,
            ContentType::Rfc6492 => rfc6492::CONTENT_TYPE,
            ContentType::Text => "text/plain",
            ContentType::Xml => "application/xml",
        }
    }
}

//----------- Response -------------------------------------------------------

struct Response {
    status: StatusCode,
    content_type: ContentType,
    body: Vec<u8>,
}

impl Response {
    fn new(status: StatusCode) -> Self {
        Response {
            status,
            content_type: ContentType::Text,
            body: Vec::new(),
        }
    }

    fn finalize(self) -> HttpResponse {
        HttpResponse(
            hyper::Response::builder()
                .status(self.status)
                .header("Content-Type", self.content_type.as_ref())
                .body(self.body.into())
                .unwrap(),
        )
    }
}

impl From<Response> for HttpResponse {
    fn from(res: Response) -> Self {
        res.finalize()
    }
}

impl io::Write for Response {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.body.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.body.flush()
    }
}

//------------ HttpResponse ---------------------------------------------------

pub struct HttpResponse(hyper::Response<Body>);

impl HttpResponse {
    fn ok_response(content_type: ContentType, body: Vec<u8>) -> Self {
        Response {
            status: StatusCode::OK,
            content_type,
            body,
        }
        .finalize()
    }

    pub fn res(self) -> Result<hyper::Response<Body>, Error> {
        Ok(self.0)
    }

    pub fn json<O: Serialize>(object: &O) -> Self {
        match serde_json::to_string(object) {
            Ok(json) => Self::ok_response(ContentType::Json, json.into_bytes()),
            Err(e) => Self::error(Error::JsonError(e)),
        }
    }

    pub fn text(body: Vec<u8>) -> Self {
        Self::ok_response(ContentType::Text, body)
    }

    pub fn xml(body: Vec<u8>) -> Self {
        Self::ok_response(ContentType::Xml, body)
    }

    pub fn rfc8181(body: Vec<u8>) -> Self {
        Self::ok_response(ContentType::Rfc8181, body)
    }

    pub fn rfc6492(body: Vec<u8>) -> Self {
        Self::ok_response(ContentType::Rfc8181, body)
    }

    pub fn cert(body: Vec<u8>) -> Self {
        Self::ok_response(ContentType::Cert, body)
    }

    pub fn error(error: Error) -> Self {
        error!("{}", error);
        let status = error.status();
        let response = error.to_error_response();
        let body = serde_json::to_string(&response).unwrap();
        Response {
            status,
            content_type: ContentType::Json,
            body: body.into_bytes(),
        }
        .finalize()
    }

    pub fn ok() -> Self {
        Response::new(StatusCode::OK).finalize()
    }

    pub fn not_found() -> Self {
        Response::new(StatusCode::NOT_FOUND).finalize()
    }

    pub fn forbidden() -> Self {
        Response::new(StatusCode::FORBIDDEN).finalize()
    }
}

//------------ Request -------------------------------------------------------

pub struct Request {
    request: hyper::Request<hyper::Body>,
    path: RequestPath,
    state: State,
}

impl Request {
    pub fn new(request: hyper::Request<hyper::Body>, state: State) -> Self {
        let path = RequestPath::from_request(&request);
        Request {
            request,
            path,
            state,
        }
    }

    /// Returns the complete path.
    pub fn path(&self) -> &RequestPath {
        &self.path
    }

    /// Get the application State
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Get a read lock on the server state
    pub fn read(&self) -> RwLockReadGuard<KrillServer> {
        self.state.read()
    }

    /// Get a write lock on the server state
    pub fn write(&self) -> RwLockWriteGuard<KrillServer> {
        self.state.write()
    }

    /// Returns the method of this request.
    pub fn method(&self) -> &Method {
        self.request.method()
    }

    /// Returns whether the request is a GET request.
    pub fn is_get(&self) -> bool {
        self.request.method() == Method::GET
    }

    /// Returns whether the request is a GET request.
    pub fn is_post(&self) -> bool {
        self.request.method() == Method::POST
    }

    /// Returns whether the request is a DELETE request.
    pub fn is_delete(&self) -> bool {
        self.request.method() == Method::DELETE
    }

    /// Get a json object from a post body
    pub async fn json<O: DeserializeOwned>(mut self) -> Result<O, Error> {
        let limit = self.read().limit_api();
        let body = self.request.into_body();

        let bytes = Self::to_bytes_limited(body, limit)
            .await
            .map_err(|_| Error::custom("Error reading body"))?;
        serde_json::from_slice(&bytes).map_err(Error::JsonError)
    }

    /// See hyper::body::to_bytes
    ///
    /// Here we want to limit the bytes consumed to a maximum. So, the
    /// code below is adapted from the method in the hyper crate.
    async fn to_bytes_limited<T>(body: T, limit: usize) -> Result<Bytes, RequestError>
    where
        T: HttpBody,
    {
        futures_util::pin_mut!(body);

        let mut size_processed = 0;

        fn assert_body_size(size: usize, limit: usize) -> Result<(), io::Error> {
            if size > limit {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Post exceeds max length",
                ))
            } else {
                Ok(())
            }
        }

        // If there's only 1 chunk, we can just return Buf::to_bytes()
        let mut first = if let Some(buf) = body.data().await {
            let buf = buf.map_err(|_| RequestError::Hyper)?;
            let size = buf.bytes().len();
            size_processed += size;
            assert_body_size(size_processed, limit)?;
            buf
        } else {
            return Ok(Bytes::new());
        };

        let second = if let Some(buf) = body.data().await {
            let buf = buf.map_err(|_| RequestError::Hyper)?;
            let size = buf.bytes().len();
            size_processed += size;
            assert_body_size(size_processed, limit)?;
            buf
        } else {
            return Ok(first.to_bytes());
        };

        // With more than 1 buf, we gotta flatten into a Vec first.
        let cap = first.remaining() + second.remaining() + body.size_hint().lower() as usize;
        let mut vec = Vec::with_capacity(cap);
        vec.put(first);
        vec.put(second);

        while let Some(buf) = body.data().await {
            let buf = buf.map_err(|_| RequestError::Hyper)?;
            let size = buf.bytes().len();
            size_processed += size;
            assert_body_size(size_processed, limit)?;

            vec.put(buf);
        }

        Ok(vec.into())
    }

    /// Checks whether the Bearer token is set to what we expect
    pub fn is_authorized(&self) -> bool {
        if let Some(header) = self.request.headers().get("Authorization") {
            if let Ok(header) = header.to_str() {
                if header.len() > 6 {
                    let (bearer, token) = header.split_at(6);
                    let bearer = bearer.trim();
                    let token = Token::from(token.trim());

                    if "Bearer" == bearer {
                        return self.read().is_api_allowed(&Auth::bearer(token));
                    }
                }
            }
        }
        false
    }
}

pub enum RequestError {
    Hyper,
    Io(io::Error),
}

impl From<io::Error> for RequestError {
    fn from(e: io::Error) -> Self {
        RequestError::Io(e)
    }
}

//------------ RequestPath ---------------------------------------------------

#[derive(Clone)]
pub struct RequestPath {
    path: PathAndQuery,
    segment: (usize, usize),
}

impl RequestPath {
    fn from_request<B>(request: &hyper::Request<B>) -> Self {
        let path = request.uri().path_and_query().unwrap().clone();
        let mut res = RequestPath {
            path,
            segment: (0, 0),
        };
        res.next_segment();
        res
    }

    pub fn full(&self) -> &str {
        self.path.path()
    }

    pub fn remaining(&self) -> &str {
        &self.full()[self.segment.1..]
    }

    pub fn segment(&self) -> &str {
        &self.full()[self.segment.0..self.segment.1]
    }

    fn next_segment(&mut self) -> bool {
        let mut start = self.segment.1;
        let path = self.full();
        // Start beyond the length of the path signals the end.
        if start >= path.len() {
            return false;
        }
        // Skip any leading slashes. There may be multiple which should be
        // folded into one (or at least that’s what we do).
        while path.split_at(start).1.starts_with('/') {
            start += 1
        }
        // Find the next slash. If we have one, that’s the end of
        // our segment, otherwise, we go all the way to the end of the path.
        let end = path[start..]
            .find('/')
            .map(|x| x + start)
            .unwrap_or(path.len());
        self.segment = (start, end);
        true
    }

    pub fn next(&mut self) -> Option<&str> {
        if self.next_segment() {
            Some(self.segment())
        } else {
            None
        }
    }

    pub fn path_arg<T>(&mut self) -> Option<T>
    where
        T: FromStr,
    {
        self.next().map(|s| T::from_str(s).ok()).flatten()
    }
}

//! Support for various admin API methods

use std::sync::{RwLockReadGuard, RwLockWriteGuard};
use actix_web::{HttpResponse, ResponseError};
use actix_web::http::StatusCode;
use serde::Serialize;
use crate::daemon::api::data::{PublisherDetails, PublisherList};
use crate::daemon::http::server::HttpRequest;
use crate::daemon::publishers;
use crate::daemon::pubserver::{self, PubServer};
use remote::oob::PublisherRequest;
use daemon::http::server::PublisherHandle;

/// Helper function to render json output.
fn render_json<O: Serialize>(object: O) -> HttpResponse {
    match serde_json::to_string(&object){
        Ok(enc) => {
            HttpResponse::Ok()
                .content_type("application/json")
                .body(enc)
        },
        Err(e) => server_error(Error::JsonError(e))
    }
}

/// Helper function to render server side errors. Also responsible for
/// logging the errors.
fn server_error(error: Error) -> HttpResponse {
    error!("{}", error);
    error.error_response()
}

/// Returns a server in a read lock
fn ro_server(req: &HttpRequest) -> RwLockReadGuard<PubServer> {
    req.state().read().unwrap()
}

/// Returns a server in a write lock
fn rw_server(req: &HttpRequest) -> RwLockWriteGuard<PubServer> {
    req.state().write().unwrap()
}

/// A clean 404 result for the API (no content, not for humans)
fn api_not_found() -> HttpResponse {
    HttpResponse::build(StatusCode::NOT_FOUND).finish()
}

/// A clean 200 result for the API (no content, not for humans)
fn api_ok() -> HttpResponse {
    HttpResponse::Ok().finish()
}

/// Returns a json structure with all publishers in it.
pub fn publishers(req: &HttpRequest) -> HttpResponse {
    match ro_server(req).publishers() {
        Err(e) => server_error(Error::ServerError(e)),
        Ok(publishers) => {
            render_json(
                PublisherList::from(&publishers, "/api/v1/publishers")
            )
        }
    }
}

/// Adds a publisher, expects that an RFC8183 section 5.2.3 Publisher
/// Request XML is posted.
pub fn add_publisher(
    req: HttpRequest,
    pr: PublisherRequest
) -> HttpResponse {
    let mut server = rw_server(&req);
    let handle = pr.handle().clone();
    match server.add_publisher(pr, &handle) {
        Ok(()) => api_ok(),
        Err(e) => server_error(Error::ServerError(e))
    }
}

pub fn add_named_publisher(
    req: HttpRequest,
    pr: PublisherRequest,
    handle: PublisherHandle
) -> HttpResponse {
    let mut server = rw_server(&req);
    match server.add_publisher(pr, handle.as_ref()) {
        Ok(()) => api_ok(),
        Err(e) => server_error(Error::ServerError(e))
    }
}

/// Removes a publisher. Should be idempotent! If if did not exist then
/// that's just fine.
pub fn remove_publisher(
    req: HttpRequest,
    handle: PublisherHandle
) -> HttpResponse {
    match rw_server(&req).remove_publisher(handle) {
        Ok(()) => api_ok(),
        Err(pubserver::Error::PublisherStoreError(
                publishers::Error::UnknownPublisher(_))) => api_ok(),
        Err(e) => server_error(Error::ServerError(e))
    }
}


/// Returns a json structure with publisher details
pub fn publisher_details(
    req: HttpRequest,
    handle: PublisherHandle
) -> HttpResponse {
    match ro_server(&req).publisher(handle) {
        Ok(None) => api_not_found(),
        Ok(Some(publisher)) => {
            render_json(
                PublisherDetails::from(&publisher, "/api/v1/publishers")
            )
        },
        Err(e) => server_error(Error::ServerError(e))
    }
}


/// Returns the id.cer for a publisher
pub fn id_cert(
    req: HttpRequest,
    handle: PublisherHandle
) -> HttpResponse {
    match ro_server(&req).publisher(handle) {
        Ok(None) => api_not_found(),
        Ok(Some(publisher)) => {
            let bytes = publisher.id_cert().to_bytes();
            HttpResponse::Ok()
                .content_type("application/pkix-cert")
                .body(bytes)
        },
        Err(e) => server_error(Error::ServerError(e))
    }
}

/// Shows the server's RFC8183 section 5.2.4 Repository Response XML
/// file for a known publisher.
pub fn repository_response(
    req: HttpRequest,
    handle: PublisherHandle
) -> HttpResponse {
    match ro_server(&req).repository_response(handle) {
        Ok(res) => {
            HttpResponse::Ok()
                .content_type("application/xml")
                .body(res.encode_vec())
        },
        Err(pubserver::Error::PublisherStoreError
            (publishers::Error::UnknownPublisher(_))) => {
            api_not_found()
        },
        Err(e) => {
            server_error(Error::ServerError(e))
        }
    }
}

//------------ Error ---------------------------------------------------------

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "{}", _0)]
    ServerError(pubserver::Error),

    #[fail(display = "{}", _0)]
    JsonError(serde_json::Error),

    #[fail(display = "Invalid publisher request")]
    PublisherRequestError
}

/// Translate an error to an HTTP Status Code
trait ErrorToStatus {
    fn status(&self) -> StatusCode;
}

/// Translate an error to an error code to include in a json response.
trait ErrorToCode {
    fn code(&self) -> usize;
}

impl ErrorToStatus for Error {
    fn status(&self) -> StatusCode {
        match self {
            Error::ServerError(e) => e.status(),
            Error::JsonError(_) => StatusCode::BAD_REQUEST,
            Error::PublisherRequestError => StatusCode::BAD_REQUEST,
        }
    }
}

impl ErrorToCode for Error {
    fn code(&self) -> usize {
        match self {
            Error::ServerError(e) => e.code(),
            Error::JsonError(_) => 1001,
            Error::PublisherRequestError => 1002,
        }
    }
}

impl ErrorToStatus for pubserver::Error {
    fn status(&self) -> StatusCode {
        match self {
            pubserver::Error::ValidationError(_) => StatusCode::FORBIDDEN,
            pubserver::Error::PublisherStoreError(e) => e.status(),
            pubserver::Error::MessageError(_) => StatusCode::BAD_REQUEST,
            pubserver::Error::RepositoryError(_) => StatusCode::BAD_REQUEST,
            pubserver::Error::ResponderError(_) => StatusCode::BAD_REQUEST,
        }
    }
}

impl ErrorToCode for pubserver::Error {
    fn code(&self) -> usize {
        match self {
            pubserver::Error::ValidationError(_) => 2001,
            pubserver::Error::PublisherStoreError(e) => e.code(),
            pubserver::Error::MessageError(_) => 1003,
            pubserver::Error::RepositoryError(_) => 3002,
            pubserver::Error::ResponderError(_) => 3003,
        }
    }
}

impl ErrorToCode for publishers::Error {
    fn code(&self) -> usize {
        match self {
            publishers::Error::ForwardSlashInHandle(_) => 1004,
            publishers::Error::DuplicatePublisher(_)   => 1005,
            publishers::Error::UnknownPublisher(_)     => 1006,
            _ => 3001
        }
    }
}

impl ErrorToStatus for publishers::Error {
    fn status(&self) -> StatusCode {
        match self {
            publishers::Error::ForwardSlashInHandle(_) =>
                StatusCode::BAD_REQUEST,
            publishers::Error::DuplicatePublisher(_) =>
                StatusCode::BAD_REQUEST,
            publishers::Error::UnknownPublisher(_) =>
                StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}


#[derive(Debug, Serialize)]
struct ErrorResponse {
    code: usize,
    msg: String
}

impl Error {
    fn to_error_response(&self) -> ErrorResponse {
        ErrorResponse {
            code: self.code(),
            msg: format!("{}", self)
        }
    }
}


impl actix_web::ResponseError for Error {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status())
            .body(serde_json::to_string(&self.to_error_response()).unwrap())
    }
}
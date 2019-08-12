//! Actix-web based HTTP server for the publication server.
//!
//! Here we deal with booting and setup, and once active deal with parsing
//! arguments and routing of requests, typically handing off to the
//! daemon::api::endpoints functions for processing and responding.
use std::fs::File;
use std::io;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use actix_session::CookieSession;
use actix_web::http::StatusCode;
use actix_web::web::{delete, get, post, scope, Path};
use actix_web::{guard, middleware, web};
use actix_web::{App, FromRequest, HttpResponse, HttpServer};
use openssl::ssl::{SslAcceptor, SslAcceptorBuilder, SslFiletype, SslMethod};

use bcder::decode;

use krill_commons::api::publication;

use crate::auth::{is_logged_in, login, logout, AUTH_COOKIE_NAME};
use crate::config::Config;
use crate::endpoints;
use crate::endpoints::*;
use crate::http::ssl;
use crate::http::statics::WithStaticContent;
use crate::krillserver;
use crate::krillserver::KrillServer;

//------------ AppServer -----------------------------------------------------

#[derive(Clone)]
pub struct AppServer(Arc<RwLock<KrillServer>>);

impl AppServer {
    pub fn read(&self) -> RwLockReadGuard<KrillServer> {
        self.0.read().unwrap()
    }

    pub fn write(&self) -> RwLockWriteGuard<KrillServer> {
        self.0.write().unwrap()
    }
}

pub fn start(config: &Config) -> Result<(), Error> {
    let server = {
        let krill = KrillServer::build(
            &config.data_dir,
            &config.rsync_base,
            config.service_uri(),
            &config.rrdp_base_uri,
            &config.auth_token,
            config.ca_refresh,
        )?;

        AppServer(Arc::new(RwLock::new(krill)))
    };

    let https_builder = https_builder(config)?;

    HttpServer::new(move || {
        App::new()
            .data(server.clone())
            .wrap(middleware::Logger::default())
            .wrap(
                CookieSession::signed(&[0; 32])
                    .name(AUTH_COOKIE_NAME)
                    .secure(true),
            )
            .route("/health", get().to(endpoints::health))
            // API end-points
            .service(
                scope("/api/v1")
                    .route("/health", get().to(api_health))
                    .route("/publishers", get().to(publishers))
                    .route("/publishers", post().to(add_publisher))
                    .route("/publishers/{handle}", get().to(publisher_details))
                    .route("/publishers/{handle}", delete().to(deactivate_publisher))
                    .route("/rfc8181/clients", get().to(rfc8181_clients))
                    .route("/rfc8181/clients", post().to(add_rfc8181_client))
                    .data(web::Bytes::configure(|cfg| cfg.limit(256 * 1024 * 1024)))
                    .route(
                        "/rfc8181/{handle}/response.xml",
                        get().to(repository_response),
                    )
                    .route("/trustanchor", get().to(ta_info))
                    .route("/trustanchor", post().to(ta_init))
                    .route("/trustanchor/children", post().to(ta_add_child))
                    .route("/trustanchor/children/{handle}", get().to(ta_show_child))
                    .route("/trustanchor/children/{handle}", post().to(ta_update_child))
                    .route("/cas", post().to(ca_init))
                    .route("/cas", get().to(cas))
                    .route("/cas/{handle}", get().to(ca_info))
                    .route("/cas/{handle}/child_request", get().to(ca_child_req))
                    .route("/cas/{handle}/parents", post().to(ca_add_parent))
                    .route("/cas/{handle}/keys/roll_init", post().to(ca_keyroll_init))
                    .route(
                        "/cas/{handle}/keys/roll_activate",
                        post().to(ca_keyroll_activate),
                    )
                    .route("/republish", post().to(republish_all)),
            )
            // Public TA related methods
            .route("/ta/ta.tal", get().to(tal))
            .route("/ta/ta.cer", get().to(ta_cer))
            // Publication by (embedded) clients
            .route("/publication/{handle}", get().to(handle_list))
            .route("/publication/{handle}", post().to(handle_delta))
            .data(web::Json::<publication::PublishDelta>::configure(|cfg| {
                cfg.limit(256 * 1024 * 1024)
            }))
            .route("/rfc8181/{handle}", post().to(rfc8181))
            // Provisioning for remote krill clients
            .route("/provisioning/{parent}/{child}/list", get().to(list))
            .route("/provisioning/{parent}/{child}/issue", post().to(issue))
            // Provisioning for rfc6492 clients
            .route("/rfc6492/{handle}", post().to(rfc6492))
            // UI support
            .route("/ui/is_logged_in", get().to(is_logged_in))
            .route("/ui/login", post().to(login))
            .route("/ui/logout", post().to(logout))
            // RRDP repository
            .route("/rrdp/{path:.*}", get().to(serve_rrdp_files))
            .route(
                "/",
                get().to(|| {
                    HttpResponse::Found()
                        .header("location", "/ui/index.html")
                        .finish()
                }),
            )
            .add_statics()
            // default
            .default_service(
                // 404 for GET request
                web::resource("")
                    .route(web::get().to(not_found))
                    // all requests that are not `GET`
                    .route(
                        web::route()
                            .guard(guard::Not(guard::Get()))
                            .to(HttpResponse::MethodNotAllowed),
                    ),
            )
    })
    .bind_ssl(config.socket_addr(), https_builder)?
    .run()?;

    Ok(())
}

/// Used to set up HTTPS. Creates keypair and self signed certificate
/// if config has 'use_ssl=test'.
fn https_builder(config: &Config) -> Result<SslAcceptorBuilder, Error> {
    if config.test_ssl() {
        ssl::create_key_cert_if_needed(&config.data_dir)
            .map_err(|e| Error::Other(format!("{}", e)))?;
    }

    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls())
        .map_err(|e| Error::Other(format!("{}", e)))?;

    builder
        .set_private_key_file(config.https_key_file(), SslFiletype::PEM)
        .map_err(|e| Error::Other(format!("{}", e)))?;

    builder
        .set_certificate_chain_file(config.https_cert_file())
        .map_err(|e| Error::Other(format!("{}", e)))?;

    Ok(builder)
}

// XXX TODO: use a better handler that does not load everything into
// memory first, and set the correct headers for caching.
// See also:
// https://github.com/actix/actix-website/blob/master/content/docs/static-files.md
// https://www.keycdn.com/blog/http-cache-headers
fn serve_rrdp_files(server: web::Data<AppServer>, path: Path<String>) -> HttpResponse {
    let mut full_path = server.read().rrdp_base_path();
    full_path.push(path.into_inner());
    match File::open(full_path) {
        Ok(mut file) => {
            use std::io::Read;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer).unwrap();

            HttpResponse::build(StatusCode::OK).body(buffer)
        }
        _ => HttpResponse::build(StatusCode::NOT_FOUND).finish(),
    }
}

//------------ Error ---------------------------------------------------------

#[derive(Debug, Display)]
#[allow(clippy::large_enum_variant)]
pub enum Error {
    #[display(fmt = "{}", _0)]
    ServerError(krillserver::Error),

    #[display(fmt = "{}", _0)]
    JsonError(serde_json::Error),

    #[display(fmt = "Cannot decode request: {}", _0)]
    DecodeError(decode::Error),

    #[display(fmt = "Wrong path")]
    WrongPath,

    #[display(fmt = "{}", _0)]
    IoError(io::Error),

    #[display(fmt = "{}", _0)]
    Other(String),
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::JsonError(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::IoError(e)
    }
}

impl From<krillserver::Error> for Error {
    fn from(e: krillserver::Error) -> Self {
        Error::ServerError(e)
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        "An error happened"
    }
}

impl actix_web::ResponseError for Error {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(format!("{}", self))
    }
}

//------------ Tests ---------------------------------------------------------

// Tested in tests/integration_test.rs

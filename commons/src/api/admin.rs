//! Support for admin tasks, such as managing publishers and RFC8181 clients

use std::fmt;

use rpki::uri;
use rpki::crypto::Signer;

use crate::api::Link;
use std::path::Path;


//------------ Handle --------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Handle(String);

impl Handle {
    pub fn as_str(&self) -> &str {
        &self.0.as_str()
    }
}

impl From<&str> for Handle {
    fn from(s: &str) -> Self {
        Handle(s.to_string())
    }
}

impl From<String> for Handle {
    fn from(s: String) -> Self {
        Handle(s)
    }
}

impl AsRef<str> for Handle {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<String> for Handle {
    fn as_ref(&self) -> &String {
        &self.0
    }
}

impl AsRef<Path> for Handle {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}


//------------ Token ------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Token(String);

impl Token {
    pub fn random<S: Signer>(signer: &S) -> Self {
        let mut res = <[u8; 20]>::default();
        signer.rand(&mut res).unwrap();
        let string = hex::encode(res);
        Token(string)
    }
}

impl From<&str> for Token {
    fn from(s: &str) -> Self {
        Token(s.to_string())
    }
}

impl From<String> for Token {
    fn from(s: String) -> Self {
        Token(s)
    }
}

impl AsRef<str> for Token {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}


//------------ PublisherRequest ----------------------------------------------

/// This type defines request for a new Publisher (CA that is allowed to
/// publish).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PublisherRequest {
    handle:   Handle,
    token:    Token,
    base_uri: uri::Rsync,
}

impl PublisherRequest {
    pub fn new(
        handle:   Handle,
        token:    Token,
        base_uri: uri::Rsync,
    ) -> Self {
        PublisherRequest {
            handle,
            token,
            base_uri,
        }
    }
}

impl PublisherRequest {
    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    pub fn token(&self) -> &Token {
        &self.token
    }

    pub fn base_uri(&self) -> &uri::Rsync {
        &self.base_uri
    }

    /// Return all the values (handle, token, base_uri).
    pub fn unwrap(self) -> (Handle, Token, uri::Rsync) {
        (self.handle, self.token, self.base_uri)
    }
}

impl PartialEq for PublisherRequest {
    fn eq(&self, other: &PublisherRequest) -> bool {
        self.handle == other.handle &&
        self.base_uri == other.base_uri
    }
}

impl Eq for PublisherRequest {}


//------------ PublisherSummaryInfo ------------------------------------------

/// Defines a summary of publisher information to be used in the publisher
/// list.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PublisherSummary {
    id: String,
    links: Vec<Link>
}

impl PublisherSummary {
    pub fn from(
        handle: &Handle,
        path_publishers: &str
    ) -> PublisherSummary {
        let mut links = Vec::new();
        let self_link = Link {
            rel: "self".to_string(),
            link: format!("{}/{}", path_publishers, handle)
        };
        links.push(self_link);

        PublisherSummary {
            id: handle.to_string(),
            links
        }
    }

    pub fn id(&self) -> &str { &self.id }
}


//------------ PublisherList -------------------------------------------------

/// This type represents a list of (all) current publishers to show in the API
#[derive(Clone, Eq, Debug, Deserialize, PartialEq, Serialize)]
pub struct PublisherList {
    publishers: Vec<PublisherSummary>
}

impl PublisherList {
    pub fn build(
        publishers: &[Handle],
        path_publishers: &str
    ) -> PublisherList {
        let publishers: Vec<PublisherSummary> = publishers.iter().map(|p|
            PublisherSummary::from(&p, path_publishers)
        ).collect();

        PublisherList {
            publishers
        }
    }

    pub fn publishers(&self) -> &Vec<PublisherSummary> {
        &self.publishers
    }
}


//------------ PublisherDetails ----------------------------------------------

/// This type defines the publisher details for:
/// /api/v1/publishers/{handle}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PublisherDetails {
    handle: String,
    deactivated: bool,
    base_uri: uri::Rsync,
}

impl PublisherDetails {
    pub fn new(handle: &str, deactivated: bool, base_uri: &uri::Rsync) -> Self {
        PublisherDetails {
            handle: handle.to_string(),
            deactivated,
            base_uri: base_uri.clone()
        }
    }

    pub fn handle(&self) -> &str { &self.handle }
    pub fn deactivated(&self) -> bool { self.deactivated }
    pub fn base_uri(&self) -> &uri::Rsync { &self.base_uri }
}

impl PartialEq for PublisherDetails {
    fn eq(&self, other: &PublisherDetails) -> bool {
        match (serde_json::to_string(self), serde_json::to_string(other)) {
            (Ok(ser_self), Ok(ser_other)) => ser_self == ser_other,
            _ => false
        }
    }
}

impl Eq for PublisherDetails {}



//------------ PublisherClientRequest ----------------------------------------

/// This type defines request for a new Publisher client, i.e. the proxy that
/// is used by an embedded CA to do the actual publication.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PublisherClientRequest {
    handle:      Handle,
    server_info: PubServerInfo
}

impl PublisherClientRequest {
    pub fn new(handle: Handle, server_info: PubServerInfo) -> Self {
        PublisherClientRequest { handle, server_info }
    }

    pub fn for_krill(
        handle: Handle,
        service_uri: uri::Https,
        token: Token
    ) -> Self {
        let server_info = PubServerInfo::for_krill(service_uri, token);
        PublisherClientRequest { handle, server_info }
    }

    pub fn unwrap(self) -> (Handle, PubServerInfo) {
        (self.handle, self.server_info)
    }
}


//------------ PubServerInfo -------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum PubServerInfo {
    KrillServer(uri::Https, Token)
}

impl PubServerInfo {
    pub fn for_krill(service_uri: uri::Https, token: Token) -> Self {
        PubServerInfo::KrillServer(service_uri, token)
    }
}
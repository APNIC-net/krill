//! An RPKI publication protocol server.
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use rpki::uri;
use krill_commons::api::publication;
use krill_commons::api::publishers;
use krill_commons::api::publishers::PublisherHandle;
use krill_commons::eventsourcing::KeyStore;
use crate::krilld::auth::Authorizer;
use crate::krilld::pubd::PubServer;
use crate::krilld::pubd;
use crate::krilld::pubd::publishers::Publisher;


//------------ KrillServer ---------------------------------------------------

/// This is the master krill server that is doing all the orchestration
/// for all the components, like:
/// * Admin tasks:
///    * Verify (admin) API access
///    * Manage known publishers
/// * CMS proxy:
///    * Decodes and validates CMS sent by known publishers using CMS
///    * Encodes and signs CMS responses for remote publishers using CMS
/// * Repository:
///    * Process publish / list requests by known publishers
///    * Updates the repository on disk
///    * Updates the RRDP files
pub struct KrillServer<S: KeyStore> {
    // The base URI for this service
    service_uri: uri::Http,

    // The base working directory, used for various storage
    work_dir: PathBuf,

    // Component responsible for API authorisation checks
    authorizer: Authorizer,

    // The configured publishers
    pubserver: PubServer<S>
}

/// # Set up and initialisation
impl<S: KeyStore> KrillServer<S> {
    /// Creates a new publication server. Note that state is preserved
    /// on disk in the work_dir provided.
    pub fn build(
        work_dir: &PathBuf,
        base_uri: &uri::Rsync,
        service_uri: uri::Http,
        rrdp_base_uri: &uri::Http,
        authorizer: Authorizer,
        store: S
    ) -> Result<Self, Error> {
        let mut repo_dir = work_dir.clone();
        repo_dir.push("repo");

        let pubserver = PubServer::build(
            base_uri.clone(),
            rrdp_base_uri.clone(),
            repo_dir,
            store
        ).map_err(Error::PubServer)?;

        Ok(
            KrillServer {
                service_uri,
                work_dir: work_dir.clone(),
                authorizer,
                pubserver
            }
        )
    }

    pub fn service_base_uri(&self) -> &uri::Http {
        &self.service_uri
    }
}

impl<S: KeyStore> KrillServer<S> {
    pub fn is_api_allowed(&self, token_opt: Option<String>) -> bool {
        self.authorizer.is_api_allowed(token_opt)
    }

    pub fn is_publication_api_allowed(
        &self,
        handle_opt: Option<String>,
        token_opt: Option<String>
    ) -> bool {
        match handle_opt {
            None => false,
            Some(handle_str) => {
                match token_opt {
                    None => false,
                    Some(token) => {
                        let handle = PublisherHandle::from(handle_str);
                        if let Ok(Some(pbl)) = self.publisher(&handle) {
                            pbl.token() == &token
                        } else {
                            false
                        }
                    }
                }
            }
        }
    }

}

/// # Configure publishers
impl<S: KeyStore> KrillServer<S> {

    /// Returns all currently configured publishers. (excludes deactivated)
    pub fn publishers(
        &self
    ) -> Result<Vec<PublisherHandle>, Error> {
        self.pubserver.list_publishers().map_err(Error::PubServer)
    }

    /// Adds the publishers, blows up if it already existed.
    pub fn add_publisher(
        &mut self,
        pbl_req: publishers::PublisherRequest
    ) -> Result<(), Error> {
        self.pubserver.create_publisher(pbl_req).map_err(Error::PubServer)
    }

    /// Removes a publisher, blows up if it didn't exist.
    pub fn deactivate_publisher(
        &mut self,
        handle: &PublisherHandle
    ) -> Result<(), Error> {
        self.pubserver.deactivate_publisher(handle).map_err(Error::PubServer)
    }

    /// Returns an option for a publisher.
    pub fn publisher(
        &self,
        handle: &PublisherHandle
    ) -> Result<Option<Arc<Publisher>>, Error> {
        self.pubserver.get_publisher(handle).map_err(Error::PubServer)
    }

    pub fn rrdp_base_path(&self) -> PathBuf {
        let mut path = self.work_dir.clone();
        path.push("rrdp");
        path
    }
}

/// # Handle publication requests
///
impl<S: KeyStore> KrillServer<S> {
    /// Handles a publish delta request sent to the API, or.. through
    /// the CmsProxy.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_delta(
        &mut self,
        delta: publication::PublishDelta,
        handle: &PublisherHandle
    ) -> Result<(), Error> {
        self.pubserver.publish(handle, delta).map_err(Error::PubServer)
    }

    /// Handles a list request sent to the API, or.. through the CmsProxy.
    pub fn handle_list(
        &self,
        handle: &PublisherHandle
    ) -> Result<publication::ListReply, Error> {
        self.pubserver.list(handle).map_err(Error::PubServer)
    }
}


//------------ Error ---------------------------------------------------------

#[derive(Debug, Display)]
#[allow(clippy::large_enum_variant)]
pub enum Error {
    #[display(fmt="{}", _0)]
    IoError(io::Error),

    #[display(fmt="{}", _0)]
    PubServer(pubd::Error),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self { Error::IoError(e) }
}

impl From<pubd::Error> for Error {
    fn from(e: pubd::Error) -> Self { Error::PubServer(e) }
}

// Tested through integration tests
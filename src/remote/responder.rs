use std::io;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use bcder::{Captured, Mode};
use bcder::encode::Values;
use rpki::crypto::{PublicKeyFormat, Signer};
use rpki::uri;
use crate::daemon::publishers::Publisher;
use crate::remote::builder;
use crate::remote::builder::{IdCertBuilder, SignedMessageBuilder};
use crate::remote::id::MyIdentity;
use crate::remote::rfc8181::Message;
use crate::remote::rfc8183::RepositoryResponse;
use crate::storage::caching_ks::CachingDiskKeyStore;
use crate::storage::keystore::{self, Info, Key, KeyStore};
use crate::util::softsigner::{self, OpenSslSigner};


/// # Naming things in the keystore.
const ACTOR: &'static str = "publication server";

fn my_id_key() -> Key {
    Key::from_str("my_id")
}

const MY_ID_MSG: &'static str = "initialised identity";


//------------ Responder -----------------------------------------------------

/// This type is responsible for managing the PubServer identity as well as
/// wrapping all response messages to publishers.
#[derive(Clone, Debug)]
pub struct Responder {
    // Used for signing responses to publishers
    signer: OpenSslSigner,

    // key value store for server specific stuff
    store: CachingDiskKeyStore,
}


/// # Set up
///
impl Responder {
    pub fn init(
        work_dir: &PathBuf,
    ) -> Result<Self, Error> {
        let mut responder_dir = PathBuf::from(work_dir);
        responder_dir.push("responder");
        if ! responder_dir.is_dir() {
            fs::create_dir_all(&responder_dir)?;
        }

        let signer = OpenSslSigner::new(&responder_dir)?;
        let store = CachingDiskKeyStore::new(responder_dir)?;

        let mut responder = Responder {
            signer,
            store,
        };
        responder.init_identity_if_empty()?;

        Ok(responder)
    }

    /// Initialise the publication server identity, if no identity had
    /// been set up. Does nothing otherwise.
    pub fn init_identity_if_empty(&mut self) -> Result<(), Error> {
        match self.my_identity()? {
            Some(_id) => Ok(()),
            None => self.init_identity()
        }
    }

    /// Initialises the identity of this publication server.
    pub fn init_identity(&mut self) -> Result<(), Error> {
        let key_id = self.signer.create_key(PublicKeyFormat)?;
        let id_cert = IdCertBuilder::new_ta_id_cert(&key_id, &mut self.signer)?;
        let my_id = MyIdentity::new(ACTOR, id_cert, key_id);

        let key = my_id_key();
        let inf = Info::now(ACTOR, MY_ID_MSG);
        self.store.store(key, my_id, inf)?;
        Ok(())
    }

    fn my_identity(&self) -> Result<Option<Arc<MyIdentity>>, Error> {
        self.store.get(&my_id_key()).map_err(|e| { Error::KeyStoreError(e)})
    }
}

/// # Provisioning
impl Responder {
    pub fn repository_response(
        &self,
        publisher: Arc<Publisher>,
        service_uri: uri::Http,
        rrdp_notification_uri: uri::Http
    ) -> Result<RepositoryResponse, Error> {
        if let Some(my_id) = self.my_identity()? {

            let tag = match publisher.cms_auth_data() {
                Some(details) => Some(details.tag().clone()),
                None => return Err(Error::ClientUnitialised)
            };


            let handle = publisher.handle();
            let id_cert = my_id.id_cert().clone();

            let sia_base = publisher.base_uri().clone();

            Ok(
                RepositoryResponse::new(
                    tag,
                    handle.clone(),
                    id_cert,
                    service_uri,
                    sia_base,
                    rrdp_notification_uri
                )
            )
        } else {
            Err(Error::Unitialised)
        }
    }

    /// Creates an encoded SignedMessage for a contained Message.
    pub fn sign_msg(&mut self, msg: Message) -> Result<Captured, Error> {
        if let Some(id) = self.my_identity()? {
            let builder = SignedMessageBuilder::new(
                id.key_id(),
                &mut self.signer,
                msg
            )?;
            let enc = builder.encode().to_captured(Mode::Der);
            Ok(enc)
        } else {
            Err(Error::Unitialised)
        }
    }
}


//------------ Error ---------------------------------------------------------

#[derive(Debug, Display)]
pub enum Error {
    #[display(fmt="{:?}", _0)]
    IoError(io::Error),

    #[display(fmt="{:?}", _0)]
    SoftSignerError(softsigner::SignerError),

    #[display(fmt="{:?}", _0)]
    KeyStoreError(keystore::Error),

    #[display(fmt="{:?}", _0)]
    BuilderError(builder::Error<softsigner::SignerError>),

    #[display(fmt="Identity of server is not initialised.")]
    Unitialised,

    #[display(fmt="Identity of client is not initialised.")]
    ClientUnitialised,
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::IoError(e)
    }
}

impl From<softsigner::SignerError> for Error {
    fn from(e: softsigner::SignerError) -> Self {
        Error::SoftSignerError(e)
    }
}

impl From<keystore::Error> for Error {
    fn from(e: keystore::Error) -> Self {
        Error::KeyStoreError(e)
    }
}

impl From<builder::Error<softsigner::SignerError>> for Error {
    fn from(e: builder::Error<softsigner::SignerError>) -> Self {
        Error::BuilderError(e)
    }
}

//------------ Tests ---------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test;
    use daemon::publishers::CmsAuthData;

    #[test]
    fn should_have_response_for_publisher() {
        test::test_with_tmp_dir(|d| {

            let responder = Responder::init(&d).unwrap();

            let name = "alice".to_string();
            let pr = test::new_publisher_request(name.as_str(), &d);
            let tag = None;
            let id_cert = pr.id_cert().clone();
            let base_uri = test::rsync_uri("rsync://host/module/alice/");
            let service_uri = test::http_uri("http://127.0.0.1:3000/rfc8181/alice");

            let rfc8181 = CmsAuthData::new(tag, id_cert);

            let publisher = Arc::new(Publisher::new(
                name,
                "token".to_string(),
                base_uri,
                Some(rfc8181)
            ));

            let rrdp_uri = test::http_uri("http://host/rrdp/");

            responder.repository_response(
                publisher,
                service_uri,
                rrdp_uri
            ).unwrap();
        });
    }

}

//! An RPKI publication protocol (command line) client, useful for testing,
//! in scenarios where a CA just writes its products to disk, and a separate
//! process is responsible for synchronising them to the repository.

use std::path::PathBuf;
use std::sync::Arc;
use std::io::Read;
use bcder::Captured;
use bcder::Mode;
use bcder::encode::Values;
use provisioning::info::ParentInfo;
use provisioning::info::MyRepoInfo;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT, CONTENT_TYPE};
use rpki::oob::exchange::PublisherRequest;
use rpki::publication::query::ListQuery;
use rpki::publication::pubmsg::Message;
use rpki::signing::builder::SignedMessageBuilder;
use rpki::signing::builder::IdCertBuilder;
use rpki::signing::PublicKeyAlgorithm;
use rpki::signing::signer::{CreateKeyError, KeyUseError, Signer};
use rpki::oob::exchange::RepositoryResponse;
use signing::identity::MyIdentity;
use signing::softsigner;
use signing::softsigner::OpenSslSigner;
use storage::caching_ks::CachingDiskKeyStore;
use storage::keystore::{self, Info, Key, KeyStore};
use reqwest::StatusCode;
use rpki::remote::sigmsg::SignedMessage;
use rpki::x509::ValidationError;
use rpki::publication::pubmsg::MessageError;
use bcder::decode;
use rpki::publication::pubmsg::ReplyMessage;
use rpki::publication::reply::ErrorReply;
use rpki::publication::reply::ListReply;
use reqwest::Response;


/// # Some constants for naming resources in the keystore for clients.
fn actor() -> String {
    "publication client".to_string()
}

fn id_key() -> Key {
    Key::from_str("my_id")
}

fn parent_key() -> Key {
    Key::from_str("my_parent")
}

fn repo_key() -> Key {
    Key::from_str("my_repo")
}

fn id_msg() -> String {
    "initialised identity".to_string()
}

fn parent_msg() -> String {
    "updated parent info".to_string()
}

fn repo_msg() -> String {
    "update repo info".to_string()
}

//------------ PubClient -----------------------------------------------------

#[derive(Clone, Debug)]
pub struct PubClient {
    // keys
    //   -> keys by id
    signer: OpenSslSigner,

    // key value store
    store: CachingDiskKeyStore,
    //   id_key     -> MyIdentity
    //   parent_key -> ParentInfo
    //   repo_key   -> MyRepoInfo

    //   -> my directory of interest
    //      (note: we do not keep this state in client, truth is on disk)
    // archive / log
    //   -> my exchanges with the server
}


impl PubClient {
    /// Creates a new publication client
    pub fn new(work_dir: &PathBuf) -> Result<Self, Error> {
        let store = CachingDiskKeyStore::new(work_dir.clone())?;
        let signer = OpenSslSigner::new(work_dir)?;
        Ok(
            PubClient {
                signer,
                store
            }
        )
    }

    /// Initialises a new publication client, using a new key pair, and
    /// returns a publisher request that can be sent to the server.
    pub fn init(&mut self, name: String) -> Result<(), Error> {
        let key_id = self.signer.create_key(&PublicKeyAlgorithm::RsaEncryption)?;
        let id_cert = IdCertBuilder::new_ta_id_cert(&key_id, &mut self.signer)?;
        let my_id = MyIdentity::new(name, id_cert, key_id);

        let key = id_key();
        let inf = Info::now(actor(), id_msg());
        self.store.store(key, my_id, inf)?;

        Ok(())
    }

    fn my_identity(&self) -> Result<Option<Arc<MyIdentity>>, Error> {
        self.store.get(&id_key()).map_err(|e| { Error::KeyStoreError(e)})
    }

    fn get_my_id(&self) -> Result<Arc<MyIdentity>, Error> {
        match self.my_identity()? {
            None => Err(Error::Uninitialised),
            Some(id) => Ok(id)
        }
    }

    fn my_parent(&self) -> Result<Option<Arc<ParentInfo>>, Error> {
        self.store.get(&parent_key()).map_err(|e| {Error::KeyStoreError(e) })
    }

    fn get_my_parent(&self) -> Result<Arc<ParentInfo>, Error> {
        match self.my_parent()? {
            None => Err(Error::Uninitialised),
            Some(p) => Ok(p)
        }
    }

    /// Process the publication server parent response.
    pub fn process_repo_response(
        &mut self,
        response: RepositoryResponse
    ) -> Result<(), Error> {

        // Store parent info
        {
            let parent_val = ParentInfo::new(
                response.publisher_handle().clone(),
                response.id_cert().clone(),
                response.service_uri().clone()
            );
            let parent_info = Info::now(actor(), parent_msg());
            let parent_key = parent_key();

            self.store.store(parent_key, parent_val, parent_info)?;
        }

        // Store repo info
        {
            let repo_val = MyRepoInfo::new(
                response.sia_base().clone(),
                response.rrdp_notification_uri().clone()
            );
            let repo_info = Info::now(actor(), repo_msg());
            let repo_key = repo_key();

            self.store.store(repo_key, repo_val, repo_info)?;
        }

        Ok(())
    }

    pub fn publisher_request(&mut self) -> Result<PublisherRequest, Error> {
        let id = self.get_my_id()?;
        Ok(
            PublisherRequest::new(
                None,
                id.name(),
                id.id_cert().clone()
            )
        )
    }

    pub fn get_server_list(&mut self) -> Result<ListReply, Error> {
        let query = ListQuery::build_message();
        let signed_request = self.sign_request(query)?;

        let reply = self.send_request(signed_request)?.as_reply()?;

        match reply {
            ReplyMessage::ErrorReply(e)   => Err(Error::ErrorReply(e)),
            ReplyMessage::SuccessReply(_) => Err(Error::UnexpectedReply),
            ReplyMessage::ListReply(l)    => Ok(l)
        }
    }

    /// Sends a signed request to the server, and validates and parses the
    /// response.
    fn send_request(&mut self, req: Captured) -> Result<Message, Error> {
        let parent = self.get_my_parent()?;

        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str("syncomator").unwrap()
        );
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_str("application/rpki-publication").unwrap()
        );

        let client = Client::new();
        let res = client.post(&parent.service_uri().to_string())
            .headers(headers)
            .body(req.to_vec())
            .send()?;

        match res.status() {
            StatusCode::OK => {
                self.parse_res(res)
            },
            _ => Err(Error::PubServerHttpError(res.status()))
        }
    }

    fn parse_res(&mut self, mut res: Response) -> Result<Message, Error> {
        let parent = self.get_my_parent()?;

        let mut bytes: Vec<u8> = vec![];
        res.read_to_end(&mut bytes).unwrap();
        let bytes = bytes::Bytes::from(bytes);

        let signed_msg = SignedMessage::decode(bytes, true)?;
        signed_msg.validate(parent.id_cert())?;
        Message::from_signed_message(&signed_msg).map_err(|e| {
            Error::MessageError(e)
        })
    }

    /// Sign a request so it can be sent to the publisher.
    fn sign_request(&mut self, msg: Message) -> Result<Captured, Error> {
        let id = self.get_my_id()?;

        let builder = SignedMessageBuilder::new(
            id.key_id(),
            &mut self.signer,
            msg
        )?;

        let enc = builder.encode().to_captured(Mode::Der);
        Ok(enc)
    }


}

// Primarily used for testing things
impl PartialEq for PubClient {
    fn eq(&self, other: &PubClient) -> bool {
        if let Ok(Some(my_id)) = self.my_identity() {
            if let Ok(Some(other_id)) = other.my_identity() {
                my_id == other_id
            } else {
                false
            }
        } else {
            false
        }
    }
}

impl Eq for PubClient { }


//------------ Error ---------------------------------------------------------

#[derive(Debug, Fail)]
pub enum Error {

    #[fail(display="This client is uninitialised.")]
    Uninitialised,

    #[fail(display="{}", _0)]
    SignerError(softsigner::Error),

    #[fail(display="{}", _0)]
    KeyStoreError(keystore::Error),

    #[fail(display="{:?}", _0)]
    CreateKeyError(CreateKeyError),

    #[fail(display="{:?}", _0)]
    KeyUseError(KeyUseError),

    #[fail(display="Received bad HTTP status code: {}", _0)]
    PubServerHttpError(StatusCode),

    #[fail(display="Request Error: {}", _0)]
    RequestError(reqwest::Error),

    #[fail(display="{}", _0)]
    ValidationError(ValidationError),

    #[fail(display="Cannot parse message: {}", _0)]
    MessageError(MessageError),

    #[fail(display="Cannot decode reply: {}", _0)]
    DecodeError(decode::Error),

    #[fail(display="Received error from server: {:?}", _0)]
    ErrorReply(ErrorReply),

    #[fail(display="Received unexpected reply (list vs success)")]
    UnexpectedReply,
}

impl From<softsigner::Error> for Error {
    fn from(e: softsigner::Error) -> Self {
        Error::SignerError(e)
    }
}

impl From<keystore::Error> for Error {
    fn from(e: keystore::Error) -> Self {
        Error::KeyStoreError(e)
    }
}

impl From<CreateKeyError> for Error {
    fn from(e: CreateKeyError) -> Self {
        Error::CreateKeyError(e)
    }
}

impl From<KeyUseError> for Error {
    fn from(e: KeyUseError) -> Self {
        Error::KeyUseError(e)
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::RequestError(e)
    }
}

impl From<ValidationError> for Error {
    fn from(e: ValidationError) -> Self {
        Error::ValidationError(e)
    }
}

impl From<decode::Error> for Error {
    fn from(e: decode::Error) -> Self {
        Error::DecodeError(e)
    }
}

impl From<MessageError> for Error {
    fn from(e: MessageError) -> Self {
        Error::MessageError(e)
    }
}



//------------ Tests ---------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use test;
    use pubd::pubserver::PubServer;

    fn test_server(work_dir: &PathBuf, xml_dir: &PathBuf) -> PubServer {
        // Start up a server
        let uri = test::rsync_uri("rsync://host/module/");
        let service = test::http_uri("http://host/publish");
        let notify = test::http_uri("http://host/notify.xml");
        PubServer::new(
            work_dir,
            xml_dir,
            &uri,
            service,
            notify
        ).unwrap()
    }

    #[test]
    fn should_initialise_keep_state_and_reinitialise() {
        test::test_with_tmp_dir(|d| {
            // Set up a new client and initialise
            let mut client_1 = PubClient::new(&d).unwrap();
            client_1.init("client".to_string()).unwrap();
            let pr_1 = client_1.publisher_request().unwrap();

            // Prove that a client starting from an initialised dir
            // comes up with the same state.
            let mut client_2 = PubClient::new(&d).unwrap();
            let pr_2 = client_2.publisher_request().unwrap();
            assert_eq!(pr_1.handle(), pr_2.handle());
            assert_eq!(pr_1.id_cert().to_bytes(), pr_2.id_cert().to_bytes());
            assert_eq!(client_1, client_2);

            // But it can be re-initialised, with a new id cert
            client_2.init("client".to_string()).unwrap();
            let pr_2 = client_2.publisher_request().unwrap();
            assert_eq!(pr_1.handle(), pr_2.handle());
            assert_ne!(pr_1.id_cert().to_bytes(), pr_2.id_cert().to_bytes());
            assert_ne!(client_1, client_2);
        });
    }

    #[test]
    fn should_process_repo_response() {
        test::test_with_tmp_dir(|d| {
            let xml_dir = test::create_sub_dir(&d);

            let alice_dir = test::create_sub_dir(&d);
            let mut alice = PubClient::new(&alice_dir).unwrap();
            alice.init("alice".to_string()).unwrap();
            let pr_alice = alice.publisher_request().unwrap();

            test::save_file(&xml_dir, "alice.xml", &pr_alice.encode_vec());

            let server = test_server(&d, &xml_dir);

            let response = server.repository_response("alice").unwrap();

            alice.process_repo_response(response).unwrap();
        });
    }

}
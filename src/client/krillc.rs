use std::time::Duration;
use clap::{App, Arg, SubCommand};
use rpki::uri;
use reqwest::{Client, StatusCode};
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use crate::client::data::{
    ApiResponse,
    PublisherList,
    ReportError,
    ReportFormat,
};

/// Command line tool for Krill admin tasks
pub struct KrillClient {
    server: uri::Http,
    token: String
}

impl KrillClient {

    /// Delegates the options to be processed, and reports the response
    /// back to the user. Note that error reporting is handled by CLI.
    pub fn report(options: Options) -> Result<(), Error> {
        let format = options.format.clone();
        let res = Self::process(options)?;

        if let Some(string) = res.report(format)? {
            println!("{}", string)
        }
        Ok(())
    }

    /// Processes the options, and returns a response ready for formatting.
    /// Note that this function is public to help integration testing the API
    /// and client.
    pub fn process(options: Options) -> Result<ApiResponse, Error> {
        let client = KrillClient {
            server: options.server,
            token:  options.token
        };

        match options.command {
            Command::Health => client.health(),
            Command::Publishers(cmd) => client.publishers(cmd),
            Command::NotSet => Err(Error::MissingCommand)
        }
    }

    /// Calls: api/v1/health
    fn health(&self) -> Result<ApiResponse, Error> {
        Self::get(&self.server, &self.token, "api/v1/health")?;
        Ok(ApiResponse::Health)
    }

    /// Calls: api/v1/publishers
    fn publishers(
        &self,
        command: PublishersCommand,
    ) -> Result<ApiResponse, Error> {
        match command {
            PublishersCommand::List => {
                let res = Self::get(
                    &self.server,
                    &self.token,
                    "api/v1/publishers")?;

                let list: PublisherList = serde_json::from_str(&res)?;

                Ok(ApiResponse::PublisherList(list))
            }
        }
    }

    /// Sends a get request to the server, including the token for
    /// authorization.
    /// Note that the server uri ends with a '/', so leave out the '/'
    /// from the start of the rel_path when calling this function.
    fn get(
        server: &uri::Http,
        token: &String,
        rel_path: &str
    ) -> Result<String, Error> {

        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str("krillc").unwrap()
        );
        headers.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap()
        );

        let client = Client::builder()
            .gzip(true)
            .timeout(Duration::from_secs(30))
            .build()?;

        let uri = format!("{}{}", server.to_string(), rel_path);
        let mut res = client.get(&uri).headers(headers).send()?;

        match res.status() {
            StatusCode::OK => {
                let txt = res.text()?;
                Ok(txt)
            },
            bad => {
                Err(Error::BadStatus(bad))
            }
        }

    }

}


/// This type holds all the necessary data to connect to a Krill daemon, and
/// authenticate, and perform a specific action. Note that this is extracted
/// from the bin/krillc.rs, so that we can use this in integration testing
/// more easily.
pub struct Options {
    server: uri::Http,
    token: String,
    format: ReportFormat,
    command: Command
}

impl Options {
    /// Creates a new Options explicitly (useful for testing)
    pub fn new(
        server: uri::Http,
        token: &str,
        format: ReportFormat,
        command: Command
    ) -> Self {
        Options { server, token: token.to_string(), format, command }
    }

    /// Creates a new Options from command line args (useful for cli)
    pub fn from_args() -> Result<Options, Error> {
        let matches = App::new("Krill admin client")
            .version("0.2.0")
            .arg(Arg::with_name("server")
                .short("s")
                .long("server")
                .value_name("URI")
                .help("Specify the full URI to the krill server.")
                .required(true))
            .arg(Arg::with_name("token")
                .short("t")
                .long("token")
                .value_name("token-string")
                .help("Specify the value of an admin token.")
                .required(true))
            .arg(Arg::with_name("format")
                .short("f")
                .long("format")
                .value_name("type")
                .help(
                    "Specify the report format (none|json|text|xml). If \
                    left unspecified the format will match the \
                    corresponding server api response type.")
                .required(false)
            )
            .subcommand(SubCommand::with_name("health")
                .about("Perform a health check. Exits with exit code 0 if \
                all is well, exit code 1 in case of any issues")
            )

            .subcommand(SubCommand::with_name("publishers")
                .about("Manage publishers")
                .subcommand(SubCommand::with_name("list")
                    .about("List all current publishers")
                )
            )

            .get_matches();

        let mut command = Command::NotSet;

        if let Some(_m) = matches.subcommand_matches("health") {
            command = Command::Health;
        }

        if let Some(m) = matches.subcommand_matches("publishers") {
            if let Some(_m) = m.subcommand_matches("list") {
                command = Command::Publishers(PublishersCommand::List)
            }
        }

        let server = matches.value_of("server").unwrap(); // required
        let server = uri::Http::from_str(server)
            .map_err(|_| Error::ServerUriError)?;

        let token = matches.value_of("token").unwrap().to_string(); // req.

        let mut format = ReportFormat::Default;
        if let Some(fmt) = matches.value_of("format") {
            format = ReportFormat::from_str(fmt)?;
        }


        Ok(Options { server, token, format, command })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    NotSet,
    Health,
    Publishers(PublishersCommand)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublishersCommand {
    List
}


//------------ Error ---------------------------------------------------------

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display ="No valid command given, see --help")]
    MissingCommand,

    #[fail(display ="Server is not available.")]
    ServerDown,

    #[fail(display ="Cannot parse server URI.")]
    ServerUriError,

    #[fail(display="Request Error: {}", _0)]
    RequestError(reqwest::Error),

    #[fail(display="Received bad status: {}", _0)]
    BadStatus(StatusCode),

    #[fail(display="Received invalid json response: {}", _0)]
    JsonError(serde_json::Error),

    #[fail(display="{}", _0)]
    ReportError(ReportError),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::RequestError(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::JsonError(e)
    }
}

impl From<ReportError> for Error {
    fn from(e: ReportError) -> Self {
        Error::ReportError(e)
    }
}


// Note: this is all tested through integration tests ('tests' folder).
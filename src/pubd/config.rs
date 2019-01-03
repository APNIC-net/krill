use std::fs::File;
use std::io;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use clap::{App, Arg};
use ext_serde;
use log::LevelFilter;
use rpki::uri;
use syslog::Facility;
use serde::de;
use serde::{Deserialize, Deserializer};
use toml;
use crate::pubd::ssl;


const SERVER_NAME: &'static str = "Publication Server";


pub struct ConfigDefaults;

impl ConfigDefaults {
    fn use_ssl() -> SslChoice { SslChoice::No }
    fn log_level() -> LevelFilter { LevelFilter::Warn }
    fn log_type() -> LogType { LogType::Syslog }
    fn syslog_facility() -> Facility { Facility::LOG_DAEMON }
    fn krill_auth_token() -> String {
        use std::env;

        match env::var("KRILL_AUTH_TOKEN") {
            Ok(token) => token,
            Err(_) => {
                eprintln!("You MUST provide a value for the master API key, either by setting \"krill_auth_token\" in the config file, or by setting the KRILL_AUTH_TOKEN environment variable.");
                ::std::process::exit(1);
            }

        }
    }
}


/// Global configuration for the RRDP Server.
///
/// This will parse a default config file ('./defaults/pubserver.conf') unless
/// another file is explicitly specified. Command line arguments may be used
/// to override any of the settings in the config file.
#[derive(Debug, Deserialize)]
pub struct Config {
    ip: IpAddr,
    port: u16,

    #[serde(default="ConfigDefaults::use_ssl")]
    use_ssl: SslChoice,

    pub data_dir: PathBuf,
    pub pub_xml_dir: PathBuf,

    #[serde(deserialize_with = "ext_serde::de_rsync_uri")]
    pub rsync_base: uri::Rsync,

    #[serde(deserialize_with = "ext_serde::de_http_uri")]
    pub rrdp_base_uri: uri::Http,

    #[serde(deserialize_with = "ext_serde::de_http_uri")]
    pub service_uri: uri::Http,

    #[serde(
        default = "ConfigDefaults::log_level",
        deserialize_with = "ext_serde::de_level_filter"
    )]
    log_level: LevelFilter,

    #[serde(default = "ConfigDefaults::log_type")]
    log_type: LogType,

    #[serde(
        default = "ConfigDefaults::syslog_facility",
        deserialize_with = "ext_serde::de_facility"
    )]
    syslog_facility: Facility,

    log_file: Option<PathBuf>,

    #[serde(default = "ConfigDefaults::krill_auth_token")]
    pub krill_auth_token: String
}

/// # Accessors
impl Config {
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.port)
    }

    pub fn use_ssl(&self) -> bool {
        self.use_ssl != SslChoice::No
    }

    pub fn test_ssl(&self) -> bool {
        self.use_ssl == SslChoice::Test
    }

    pub fn https_cert_file(&self) -> PathBuf {
        let mut path = self.data_dir.clone();
        path.push(ssl::HTTPS_SUB_DIR);
        path.push(ssl::CERT_FILE);
        path
    }

    pub fn https_key_file(&self) -> PathBuf {
        let mut path = self.data_dir.clone();
        path.push(ssl::HTTPS_SUB_DIR);
        path.push(ssl::KEY_FILE);
        path
    }
}

/// # Create
impl Config {
    /// Set up a config for use in (integration) testing.
//    #[cfg(test)]
    pub fn test(
        data_dir: &PathBuf,
        pub_xml_dir: &PathBuf,
    ) -> Self {
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        ;
        let port = 3000;
        let use_ssl = SslChoice::No;
        let data_dir = data_dir.clone();
        let pub_xml_dir = pub_xml_dir.clone();
        let rsync_base = uri::Rsync::from_str("rsync://127.0.0.1/rpki/")
            .unwrap();
        let rrdp_base_uri = uri::Http::from_str(
            "http://127.0.0.1:3000/rrdp/").unwrap();
        let service_uri = uri::Http::from_str(
            "http://127.0.0.1:3000/rfc8181/").unwrap();
        let log_level = ConfigDefaults::log_level();
        let log_type = ConfigDefaults::log_type();
        let log_file = None;
        let syslog_facility = ConfigDefaults::syslog_facility();
        let krill_auth_token = "secret".to_string();

        Config {
            ip,
            port,
            use_ssl,
            data_dir,
            pub_xml_dir,
            rsync_base,
            rrdp_base_uri,
            service_uri,
            log_level,
            log_type,
            log_file,
            syslog_facility,
            krill_auth_token
        }
    }

    /// Creates the config (at startup). Panics in case of issues.
    pub fn create() -> Result<Self, ConfigError> {
        let matches = App::new("NLnet Labs RRDP Server")
            .version("0.1b")
            .arg(Arg::with_name("config")
                .short("c")
                .long("config")
                .value_name("FILE")
                .help("Specify non-default config file. If no file is \
                specified './defaults/pubserver.conf' will be used to \
                determine default values for all settings. Note that you \
                can use any of the following options to override any of \
                these values..")
                .required(false))
            .get_matches();

        let config_file = matches.value_of("config")
            .unwrap_or("./defaults/pubserver.conf");

        let c = Self::read_config(config_file.as_ref())?;
        c.init_logging()?;
        Ok(c)
    }

    fn read_config(file: &str) -> Result<Self, ConfigError> {
        let mut v = Vec::new();
        let mut f = File::open(file)?;
        f.read_to_end(&mut v)?;

        let c: Config = toml::from_slice(v.as_slice())?;

        if c.port < 1024 {
            return Err(ConfigError::from_str("Port number must be >1024"))
        }

        if c.log_type == LogType::File && c.log_file == None {
            return Err(ConfigError::from_str(
                "Must specify log_file if log_type is 'file'."
            ))
        }

        Ok(c)
    }

    pub fn init_logging(&self) -> Result<(), ConfigError> {
        match self.log_type {
            LogType::File => {
                let file = fern::log_file(self.log_file.as_ref().unwrap())?;

                let mut dispatch = fern::Dispatch::new();

                dispatch = {
                    if self.log_level == LevelFilter::Debug {
                        dispatch.format(|out, message, record| {
                            out.finish(
                                format_args!(
                                    "{} [{}] [{}] {}",
                                    chrono::Local::now()
                                        .format("%Y-%m-%d %H:%M:%S"),
                                    record.target(),
                                    record.level(),
                                    message
                                )
                            )
                        })
                    } else {
                        dispatch.format(|out, message, record| {
                            out.finish(
                                format_args!(
                                    "{} [{}] {}",
                                    chrono::Local::now()
                                        .format("%Y-%m-%d %H:%M:%S"),
                                    record.level(),
                                    message
                                )
                            )
                        })
                    }
                };

                dispatch.level(self.log_level)
                    .chain(file)
                    .apply()
                    .map_err(|e| {
                        ConfigError::Other(
                            format!("Failed to init file logging: {}", e)
                        )
                    })?;
            },

            LogType::Syslog => {
                syslog::init(
                    self.syslog_facility,
                    self.log_level,
                    Some(SERVER_NAME)
                ).map_err(|e| {
                    ConfigError::Other(
                        format!("Failed to init syslog: {}", e)
                    )
                })?;
            },

            LogType::Stderr => {
                let dispatch = fern::Dispatch::new()
                    .level(self.log_level)
                    .chain(io::stderr());

                dispatch.apply().map_err(|e| {
                    ConfigError::Other(
                        format!("Failed to init stderr logging: {}", e)
                    )
                })?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Fail)]
pub enum ConfigError {

    #[fail(display ="{}", _0)]
    IoError(io::Error),

    #[fail(display ="{}", _0)]
    TomlError(toml::de::Error),

    #[fail(display ="{}", _0)]
    RpkiUriError(uri::Error),

    #[fail(display ="{}", _0)]
    Other(String)
}

impl ConfigError {
    pub fn from_str(s: &str) -> ConfigError {
        ConfigError::Other(s.to_string())
    }
}

impl From<io::Error> for ConfigError {
    fn from(e: io::Error) -> Self {
        ConfigError::IoError(e)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(e: toml::de::Error) -> Self {
        ConfigError::TomlError(e)
    }
}

impl From<uri::Error> for ConfigError {
    fn from(e: uri::Error) -> Self {
        ConfigError::RpkiUriError(e)
    }
}


//------------ LogType -------------------------------------------------------

/// The target to log to.
#[derive(Clone, Debug)]
pub enum LogType {
    Syslog,
    Stderr,
    File
}


//--- PartialEq and Eq

impl PartialEq for LogType {
    fn eq(&self, other: &LogType) -> bool {
        match (self, other) {
            (&LogType::Syslog, &LogType::Syslog) => true,
            (&LogType::Stderr, &LogType::Stderr) => true,
            (&LogType::File, &LogType::File) => true,
            _ => false
        }
    }
}

impl Eq for LogType { }

impl<'de> Deserialize<'de> for LogType {
    fn deserialize<D>(d: D) -> Result<LogType, D::Error>
        where D: Deserializer<'de> {
        let string = String::deserialize(d)?;
        match string.as_str() {
            "stderr" => Ok(LogType::Stderr),
            "syslog" => Ok(LogType::Syslog),
            "file" => Ok(LogType::File),
            _ => Err(
                    de::Error::custom(
                        format!("expected \"stderr\", \"syslog\", or \
                        \"file\", found : \"{}\"", string)))
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SslChoice {
    No,
    Yes,
    Test
}

impl<'de> Deserialize<'de> for SslChoice {
    fn deserialize<D>(d: D) -> Result<SslChoice, D::Error>
        where D: Deserializer<'de> {
        let string = String::deserialize(d)?;
        match string.as_str() {
            "no"   => Ok(SslChoice::No),
            "yes"  => Ok(SslChoice::Yes),
            "test" => Ok(SslChoice::Test),
            _ => Err(
                de::Error::custom(
                    format!("expected \"yes\", \"no\" or \"test\", \
                    found: \"{}\"", string)))
        }
    }
}


//------------ Tests ---------------------------------------------------------

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn should_parse_default_config_file() {
        // Config for auth token is required! If there is nothing in the conf
        // file, then an environment variable must be set.
        use std::env;
        env::set_var("KRILL_AUTH_TOKEN", "secret");

        let c = Config::read_config("./defaults/pubserver.conf").unwrap();
        let expected_socket_addr = ([127, 0, 0, 1], 3000).into();
        assert_eq!(c.socket_addr(), expected_socket_addr);
    }

}

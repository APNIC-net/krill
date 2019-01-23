//! Data types to wrap the API responses, and support reporting on them in
//! various formats (where applicable).
use rpki::uri;
use crate::util::ext_serde;
use remote::id::IdCert;

//------------ ApiResponse ---------------------------------------------------

/// This type defines all supported responses for the api
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApiResponse {
    Health,
    PublisherDetails(PublisherDetails),
    PublisherList(PublisherList),
    Empty, // Typically a successful post just gets an empty 200 response
    GenericBody(String) // For when the server echos Json to a successful post
}

impl ApiResponse {
    pub fn report(
        &self,
        fmt: ReportFormat
    ) -> Result<Option<String>, ReportError> {
        if fmt == ReportFormat::None {
            Ok(None)
        } else {
            match self {
                ApiResponse::Health => {
                    if fmt == ReportFormat::Default {
                        Ok(None)
                    } else {
                        Err(ReportError::UnsupportedFormat)
                    }
                },
                ApiResponse::PublisherList(list) => {
                    Ok(Some(list.report(fmt)?))
                },
                ApiResponse::PublisherDetails(details) => {
                    Ok(Some(details.report(fmt)?))
                }
                ApiResponse::GenericBody(body) => {
                    Ok(Some(body.clone()))
                }
                ApiResponse::Empty => Ok(None)
            }
        }
    }
}

//------------ ReportFormat --------------------------------------------------

/// This type defines the format to use when representing the api response
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReportFormat {
    Default, // the normal format for this data type
    None,
    Json,
    Text,
    Xml
}

impl ReportFormat {
    pub fn from_str(s: &str) -> Result<Self, ReportError> {
        match s {
            "none" => Ok(ReportFormat::None),
            "json" => Ok(ReportFormat::Json),
            "text" => Ok(ReportFormat::Text),
            "xml"  => Ok(ReportFormat::Xml),
            _ => Err(ReportError::UnrecognisedFormat(s.to_string()))
        }
    }
}


//------------ ReportError ---------------------------------------------------

/// This type defines possible Errors for KeyStore
#[derive(Debug, Display)]
pub enum ReportError {
    #[display(fmt="This report format is not supported for this data")]
    UnsupportedFormat,

    #[display(fmt="This report format is not recognised: {}", _0)]
    UnrecognisedFormat(String)
}


//------------ Report --------------------------------------------------------

/// This trait should be implemented by all api responses, so that the
/// response can be formatted for users.
trait Report {
    fn report(&self, format: ReportFormat) -> Result<String, ReportError>;
}


//------------ Link ----------------------------------------------------------

/// This type defines a json link item, often included in json responses as
/// helpful hints for more..
#[derive(Clone, Debug, Deserialize, Eq, Serialize, PartialEq)]
pub struct Link {
    rel: String,
    link: String
}


//------------ PublisherList -------------------------------------------------

/// This type defines the response for:
/// /api/v1/publishers
#[derive(Clone, Debug, Deserialize, Eq, Serialize, PartialEq)]
pub struct PublisherList {
    publishers: Vec<PublisherSummary>
}

impl PublisherList {
    pub fn publishers(&self) -> &Vec<PublisherSummary> {
        &self.publishers
    }
}

impl Report for PublisherList {
    fn report(&self, format: ReportFormat) -> Result<String, ReportError> {
        match format {
            ReportFormat::Default | ReportFormat::Json => {
                Ok(serde_json::to_string(self).unwrap())
            },
            ReportFormat::Text => {
                let mut res = String::new();

                res.push_str("Publishers: ");
                let mut first = true;
                for p in &self.publishers {
                    if ! first {
                        res.push_str(", ");
                    } else {
                        first = false;
                    }
                    res.push_str(p.id());
                }
                Ok(res)
            },
            _ => Err(ReportError::UnsupportedFormat)
        }
    }
}


//------------ PublisherSummary ----------------------------------------------

/// This type defines an individual publisher in the response for:
/// /api/v1/publishers
#[derive(Clone, Debug, Deserialize, Eq, Serialize, PartialEq)]
pub struct PublisherSummary {
    id: String,
    links: Vec<Link>
}

impl PublisherSummary {
    pub fn id(&self) -> &String {
        &self.id
    }
}


//------------ Rfc8181Details ------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CmsAuthData {
    #[serde(
        serialize_with = "ext_serde::ser_http_uri",
        deserialize_with = "ext_serde::de_http_uri")]
    service_uri: uri::Http,

    #[serde(
        serialize_with = "ext_serde::ser_id_cert",
        deserialize_with = "ext_serde::de_id_cert")]
    id_cert: IdCert
}


//------------ PublisherDetails ----------------------------------------------

/// This type defines the publisher details fro:
/// /api/v1/publishers/{handle}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PublisherDetails {
    publisher_handle: String,

    #[serde(
        deserialize_with = "ext_serde::de_rsync_uri",
        serialize_with = "ext_serde::ser_rsync_uri"
    )]
    base_uri: uri::Rsync,

    cms_auth: Option<CmsAuthData>,

    links: Vec<Link>
}

impl PublisherDetails {
    pub fn publisher_handle(&self) -> &str {
        &self.publisher_handle
    }
    pub fn identity_cert(&self) -> Option<&IdCert> {
        match self.cms_auth {
            None => None,
            Some(ref details) => Some(&details.id_cert)
        }
    }
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

impl Report for PublisherDetails {
    fn report(&self, format: ReportFormat) -> Result<String, ReportError> {
        match format {
            ReportFormat::Default | ReportFormat::Json => {
                Ok(serde_json::to_string(self).unwrap())
            },
            ReportFormat::Text => {

                let mut res = String::new();

                res.push_str("handle: ");
                res.push_str(self.publisher_handle.as_str());
                res.push_str("\n");

                res.push_str("base uri: ");
                res.push_str(self.base_uri.to_string().as_str());
                res.push_str("\n");

                if let Some(ref rfc8181) = self.cms_auth {
                    res.push_str("RFC8181 Details:\n");
                    res.push_str("  service uri: ");
                    res.push_str(rfc8181.service_uri.to_string().as_str());
                    res.push_str("\n");
                    res.push_str("  id cert (base64): ");
                    res.push_str(
                        base64::encode(
                            rfc8181.id_cert.to_bytes().as_ref()
                        ).as_str());
                    res.push_str("\n");
                }

                Ok(res)
            },
            _ => Err(ReportError::UnsupportedFormat)
        }
    }
}
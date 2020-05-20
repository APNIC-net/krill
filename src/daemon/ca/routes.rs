use std::collections::{HashMap, HashSet};
use std::fmt;
use std::net::IpAddr;
use std::ops::Deref;
use std::str::FromStr;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use rpki::resources;
use rpki::roa::{Roa, RoaBuilder};
use rpki::sigobj::SignedObjectBuilder;
use rpki::uri;
use rpki::x509::{Serial, Time};

use crate::commons::api::{
    CurrentObject, ObjectName, ReplacedObject, RoaDefinition, RoaDefinitionUpdates,
};
use crate::commons::KrillResult;
use crate::commons::error::Error;
use crate::commons::api::{AsNumber, TypedPrefix};
use crate::daemon::ca::events::RoaUpdates;
use crate::daemon::ca::{self, CertifiedKey, SignSupport, Signer};

//------------ RouteAuthorization ------------------------------------------

/// This type defines a prefix and optional maximum length (other than the
/// prefix length) which is to be authorized for the given origin ASN.
#[derive(Clone, Copy, Debug, Display, Eq, Hash, PartialEq)]
pub struct RouteAuthorization(RoaDefinition);

impl RouteAuthorization {
    pub fn new(definition: RoaDefinition) -> Self {
        RouteAuthorization(definition)
    }
}

impl AsRef<RoaDefinition> for RouteAuthorization {
    fn as_ref(&self) -> &RoaDefinition {
        &self.0
    }
}

impl Deref for RouteAuthorization {
    type Target = RoaDefinition;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// We use RouteAuthorization as (json) map keys and therefore we need it
/// to be serializable to a single simple string.
impl Serialize for RouteAuthorization {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.to_string().serialize(s)
    }
}

/// We use RouteAuthorization as (json) map keys and therefore we need it
/// to be deserializable from a single simple string.
impl<'de> Deserialize<'de> for RouteAuthorization {
    fn deserialize<D>(d: D) -> Result<RouteAuthorization, D::Error>
    where
        D: Deserializer<'de>,
    {
        let string = String::deserialize(d)?;
        let def = RoaDefinition::from_str(string.as_str()).map_err(de::Error::custom)?;
        Ok(RouteAuthorization(def))
    }
}

impl From<RoaDefinition> for RouteAuthorization {
    fn from(def: RoaDefinition) -> Self {
        RouteAuthorization(def)
    }
}

//------------ RouteAuthorizationUpdates -----------------------------------

///
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteAuthorizationUpdates {
    added: HashSet<RouteAuthorization>,
    removed: HashSet<RouteAuthorization>,
}

impl RouteAuthorizationUpdates {
    pub fn unpack(self) -> (HashSet<RouteAuthorization>, HashSet<RouteAuthorization>) {
        (self.added, self.removed)
    }
}

impl From<RoaDefinitionUpdates> for RouteAuthorizationUpdates {
    fn from(definitions: RoaDefinitionUpdates) -> Self {
        let (added, removed) = definitions.unpack();
        let added = added.into_iter().map(RoaDefinition::into).collect();
        let removed = removed.into_iter().map(RoaDefinition::into).collect();
        RouteAuthorizationUpdates { added, removed }
    }
}

impl fmt::Display for RouteAuthorizationUpdates {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if !self.added.is_empty() {
            write!(f, "added:")?;
            for a in &self.added {
                write!(f, " {}", a)?;
            }
            write!(f, " ")?;
        }
        if !self.removed.is_empty() {
            write!(f, "removed:")?;
            for r in &self.removed {
                write!(f, " {}", r)?;
            }
        }
        Ok(())
    }
}

//------------ Routes ------------------------------------------------------

/// The current authorizations and corresponding meta-information for a CA.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Routes {
    map: HashMap<RouteAuthorization, RouteInfo>,
}

impl Default for Routes {
    fn default() -> Self {
        Routes {
            map: HashMap::new(),
        }
    }
}

impl Routes {
    pub fn authorizations(&self) -> impl Iterator<Item = &RouteAuthorization> {
        self.map.keys()
    }

    pub fn info(&self, auth: &RouteAuthorization) -> Option<&RouteInfo> {
        self.map.get(auth)
    }

    pub fn has(&self, auth: &RouteAuthorization) -> bool {
        self.map.contains_key(auth)
    }

    /// Adds a new authorization, or updates an existing one.
    pub fn add(&mut self, auth: RouteAuthorization) {
        self.map.insert(auth, RouteInfo::default());
    }

    /// Removes an authorization
    pub fn remove(&mut self, auth: &RouteAuthorization) {
        self.map.remove(auth);
    }
}

//------------ RouteInfo ---------------------------------------------------

/// Meta-information about a configured RouteAuthorization.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RouteInfo {
    since: Time, // authorization first added by user
}

impl Default for RouteInfo {
    fn default() -> Self {
        RouteInfo { since: Time::now() }
    }
}

//------------ RoaInfo -----------------------------------------------------

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoaInfo {
    object: CurrentObject,            // actual ROA
    name: ObjectName,                 // Name for object in repo
    since: Time,                      // first ROA in RC created
    replaces: Option<ReplacedObject>, // for revoking when re-newing
}

impl RoaInfo {
    pub fn new_roa(roa: &Roa, name: ObjectName) -> Self {
        let object = CurrentObject::from(roa);
        RoaInfo {
            object,
            name,
            since: Time::now(),
            replaces: None,
        }
    }

    pub fn updated_roa(old: &RoaInfo, roa: &Roa, name: ObjectName) -> Self {
        let object = CurrentObject::from(roa);
        let replaces = Some(ReplacedObject::from(old.object()));
        RoaInfo {
            object,
            name,
            since: old.since,
            replaces,
        }
    }

    pub fn object(&self) -> &CurrentObject {
        &self.object
    }

    pub fn name(&self) -> &ObjectName {
        &self.name
    }

    pub fn since(&self) -> Time {
        self.since
    }

    pub fn replaces(&self) -> Option<&ReplacedObject> {
        self.replaces.as_ref()
    }
}

// Utility methods
impl RoaInfo {
    pub fn retrieve_route_authorizations(&self) -> KrillResult<Vec<RouteAuthorization>> {
        let roa = Roa::decode(self.object.content().to_bytes(), false)
            .map_err(|_| Error::Custom(format!("Error decoding ROA {}", self.name)))?;

        let mut auths: Vec<RouteAuthorization> = Vec::new();
        let roa_content = roa.content();
        let asn = AsNumber::new(u32::from(roa_content.as_id()));

        for fria in roa_content.iter() {
            let (prefix, max_length) = RoaInfo::route_authorization_from(&fria);
            auths.push(RouteAuthorization(RoaDefinition::new(asn, prefix, max_length)));
        }

        Ok(auths)
    }

    fn route_authorization_from(fria: &rpki::roa::FriendlyRoaIpAddress) -> (TypedPrefix, Option<u8>)  {
        let address = fria.address();
        let addr_len = fria.address_length();
        let max_length = if addr_len == fria.max_length() { Option::None } else { Option::Some(fria.max_length()) };
        let prefix = match address {
            IpAddr::V4(v4) => TypedPrefix::v4_from_prefix(resources::Prefix::new(resources::Addr::from_v4(v4), addr_len)),
            IpAddr::V6(v6) => TypedPrefix::v6_from_prefix(resources::Prefix::new(resources::Addr::from_v6(v6), addr_len)),
        };

        (prefix, max_length)
    }
}


//------------ Roas --------------------------------------------------------

/// ROAs held by a resource class in a CA.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Roas {
    inner: HashMap<RouteAuthorization, RoaInfo>,
}

impl Default for Roas {
    fn default() -> Self {
        Roas {
            inner: HashMap::new(),
        }
    }
}

impl Roas {
    pub fn get(&self, auth: &RouteAuthorization) -> Option<&RoaInfo> {
        self.inner.get(auth)
    }

    pub fn updated(&mut self, updates: RoaUpdates) {
        let (updated, removed) = updates.unpack();

        for (auth, info) in updated.into_iter() {
            self.inner.insert(auth, info);
        }

        for auth in removed.keys() {
            self.inner.remove(auth);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&RouteAuthorization, &RoaInfo)> {
        self.inner.iter()
    }

    pub fn current(&self) -> impl Iterator<Item = &RoaInfo> {
        self.inner.values()
    }

    pub fn authorizations(&self) -> impl Iterator<Item = &RouteAuthorization> {
        self.inner.keys()
    }

    pub fn make_roa<S: Signer>(
        auth: &RouteAuthorization,
        certified_key: &CertifiedKey,
        new_repo: Option<&uri::Rsync>,
        signer: &S,
    ) -> KrillResult<Roa> {
        let mut auths = HashSet::with_capacity(1);
        auths.insert(*auth);
        Roas::make_roa_multi(&auths, certified_key, new_repo, signer, auth)
    }

    pub fn make_roa_multi<S: Signer>(
        auths: &HashSet<RouteAuthorization>,
        certified_key: &CertifiedKey,
        new_repo: Option<&uri::Rsync>,
        signer: &S,
        name: impl Into<ObjectName>,
    ) -> KrillResult<Roa> {
        assert!(auths.len() > 0);
        let _auth = *auths.iter().take(1).next().unwrap();
        let asn = _auth.asn();

        let incoming_cert = certified_key.incoming_cert();
        let crl_uri = match &new_repo {
            None => incoming_cert.crl_uri(),
            Some(base_uri) => base_uri.join(incoming_cert.crl_name().as_bytes()),
        };

        let roa_uri = match &new_repo {
            None => incoming_cert.uri_for_object(name),
            Some(base_uri) => base_uri.join((name.into() as ObjectName).as_bytes()),
        };

        let aia = incoming_cert.uri();

        let signing_key = certified_key.key_id();

        let mut roa_builder = RoaBuilder::new(asn.into());
        auths.iter().for_each(|auth| { 
            let prefix = auth.prefix();
            roa_builder.push_addr(prefix.ip_addr(), prefix.addr_len(), auth.max_length());
        });
        let mut object_builder = SignedObjectBuilder::new(
            Serial::random(signer).map_err(ca::Error::signer)?,
            SignSupport::sign_validity_year(),
            crl_uri,
            aia.clone(),
            roa_uri,
        );
        object_builder.set_issuer(Some(incoming_cert.cert().subject().clone()));
        object_builder.set_signing_time(Some(Time::now()));

        roa_builder
            .finalize(object_builder, signer, signing_key)
            .map_err(ca::Error::signer)
    }
}

//------------ Tests -------------------------------------------------------

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn serde_route_authorization() {
        fn parse_encode_authorization(s: &str) {
            let def = RoaDefinition::from_str(s).unwrap();
            let auth = RouteAuthorization(def);

            let json = serde_json::to_string(&auth).unwrap();
            assert_eq!(format!("\"{}\"", s), json);

            let des: RouteAuthorization = serde_json::from_str(&json).unwrap();
            assert_eq!(des, auth);
        }

        parse_encode_authorization("192.168.0.0/16 => 64496");
        parse_encode_authorization("192.168.0.0/16-24 => 64496");
        parse_encode_authorization("2001:db8::/32 => 64496");
        parse_encode_authorization("2001:db8::/32-48 => 64496");
    }
}

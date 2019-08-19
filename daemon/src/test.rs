//! Support for tests in other modules using a running krill server

use std::path::PathBuf;
use std::{thread, time};

use krill_client::options::{CaCommand, Command, Options, TrustAnchorCommand};
use krill_client::report::{ApiResponse, ReportFormat};
use krill_client::{Error, KrillClient};

use krill_commons::api::admin::{
    AddChildRequest, AddParentRequest, CertAuthInit, CertAuthPubMode, ChildAuthRequest, Handle,
    ParentCaContact, Token, UpdateChildRequest,
};
use krill_commons::api::ca::{CertAuthInfo, ResourceClassKeysInfo, ResourceClassName, ResourceSet};
use krill_commons::remote::rfc8183;
use krill_commons::util::test;

use crate::ca::ta_handle;
use crate::config::Config;
use crate::http::server;

pub fn test_with_krill_server<F>(op: F)
where
    F: FnOnce(PathBuf) -> (),
{
    test::test_under_tmp(|dir| {
        // Set up a test PubServer Config
        let server_conf = {
            // Use a data dir for the storage
            let data_dir = test::sub_dir(&dir);
            Config::test(&data_dir)
        };

        // Start the server
        thread::spawn(move || server::start(&server_conf).unwrap());

        let mut tries = 0;
        loop {
            thread::sleep(time::Duration::from_millis(100));
            if let Ok(_res) = health_check() {
                break;
            }

            tries += 1;
            if tries > 20 {
                panic!("Server is not coming up")
            }
        }

        op(dir)
    })
}

pub fn wait_seconds(s: u64) {
    thread::sleep(time::Duration::from_secs(s));
}

fn health_check() -> Result<ApiResponse, Error> {
    let krillc_opts = Options::new(
        test::https("https://localhost:3000/"),
        "secret",
        ReportFormat::Default,
        Command::Health,
    );

    KrillClient::process(krillc_opts)
}

pub fn krill_admin(command: Command) -> ApiResponse {
    let krillc_opts = Options::new(
        test::https("https://localhost:3000/"),
        "secret",
        ReportFormat::Json,
        command,
    );
    match KrillClient::process(krillc_opts) {
        Ok(res) => res, // ok
        Err(e) => panic!("{}", e),
    }
}

pub fn init_ta() {
    krill_admin(Command::TrustAnchor(TrustAnchorCommand::Init));
}

pub fn init_child(handle: &Handle, token: &Token) {
    let init = CertAuthInit::new(handle.clone(), token.clone(), CertAuthPubMode::Embedded);
    krill_admin(Command::CertAuth(CaCommand::Init(init)));
}

pub fn child_request(handle: &Handle) -> rfc8183::ChildRequest {
    match krill_admin(Command::CertAuth(CaCommand::ChildRequest(handle.clone()))) {
        ApiResponse::Rfc8183ChildRequest(req) => req,
        _ => panic!("Expected child request"),
    }
}

pub fn add_child_to_ta_embedded(handle: &Handle, resources: ResourceSet) -> ParentCaContact {
    let auth = ChildAuthRequest::Embedded;
    let req = AddChildRequest::new(handle.clone(), resources, auth);
    let res = krill_admin(Command::TrustAnchor(TrustAnchorCommand::AddChild(req)));

    match res {
        ApiResponse::ParentCaInfo(info) => info,
        _ => panic!("Expected ParentCaInfo response"),
    }
}

pub fn add_child_to_ta_rfc6492(
    handle: &Handle,
    req: rfc8183::ChildRequest,
    resources: ResourceSet,
) -> ParentCaContact {
    let auth = ChildAuthRequest::Rfc8183(req);
    let req = AddChildRequest::new(handle.clone(), resources, auth);
    let res = krill_admin(Command::TrustAnchor(TrustAnchorCommand::AddChild(req)));

    match res {
        ApiResponse::ParentCaInfo(info) => info,
        _ => panic!("Expected ParentCaInfo response"),
    }
}

pub fn update_child(handle: &Handle, resources: &ResourceSet) {
    let req = UpdateChildRequest::graceful(None, Some(resources.clone()));
    match krill_admin(Command::TrustAnchor(TrustAnchorCommand::UpdateChild(
        handle.clone(),
        req,
    ))) {
        ApiResponse::Empty => {}
        _ => panic!("Expected empty ok response"),
    }
}

pub fn force_update_child(handle: &Handle, resources: &ResourceSet) {
    let req = UpdateChildRequest::force(None, Some(resources.clone()));
    match krill_admin(Command::TrustAnchor(TrustAnchorCommand::UpdateChild(
        handle.clone(),
        req,
    ))) {
        ApiResponse::Empty => {}
        _ => panic!("Expected empty ok response"),
    }
}

pub fn add_parent_to_ca(handle: &Handle, parent: AddParentRequest) {
    krill_admin(Command::CertAuth(CaCommand::AddParent(
        handle.clone(),
        parent,
    )));
}

pub fn ca_roll_init(handle: &Handle) {
    krill_admin(Command::CertAuth(CaCommand::KeyRollInit(handle.clone())));
}

pub fn ca_roll_activate(handle: &Handle) {
    krill_admin(Command::CertAuth(CaCommand::KeyRollActivate(
        handle.clone(),
    )));
}

pub fn ca_details(handle: &Handle) -> CertAuthInfo {
    match krill_admin(Command::CertAuth(CaCommand::Show(handle.clone()))) {
        ApiResponse::CertAuthInfo(inf) => inf,
        _ => panic!("Expected cert auth info"),
    }
}

pub fn wait_for<O>(tries: u64, error_msg: &'static str, op: O)
where
    O: Copy + FnOnce() -> bool,
{
    for _counter in 1..=tries {
        if op() {
            return;
        }
        wait_seconds(1);
    }
    panic!(error_msg);
}

pub fn wait_for_resources_on_current_key(handle: &Handle, resources: &ResourceSet) {
    wait_for(
        30,
        "cms child did not get its resource certificate",
        move || &ca_current_resources(handle) == resources,
    )
}

pub fn wait_for_new_key(handle: &Handle) {
    wait_for(30, "No new key received", move || {
        let ca = ca_details(handle);
        if let Some(parent) = ca.parent(&ta_handle()) {
            if let Some(rc) = parent.resources().get(&ResourceClassName::default()) {
                match rc.keys() {
                    ResourceClassKeysInfo::RollNew(new, _) => {
                        return new.current_set().number() == 2
                    }
                    _ => return false,
                }
            }
        }

        false
    })
}

pub fn wait_for_key_roll_complete(handle: &Handle) {
    wait_for(30, "Key roll did not complete", || {
        let ca = ca_details(handle);

        if let Some(parent) = ca.parent(&ta_handle()) {
            if let Some(rc) = parent.resources().get(&ResourceClassName::default()) {
                match rc.keys() {
                    ResourceClassKeysInfo::Active(_) => return true,
                    _ => return false,
                }
            }
        }

        false
    })
}

pub fn wait_for_resource_class_to_disappear(handle: &Handle) {
    wait_for(30, "Resource class not removed", || {
        let ca = ca_details(handle);

        if let Some(parent) = ca.parent(&ta_handle()) {
            return parent
                .resources()
                .get(&ResourceClassName::default())
                .is_none();
        }

        false
    })
}

pub fn wait_for_ta_to_have_number_of_issued_certs(number: usize) {
    wait_for(30, "TA has wrong amount of issued certs", || {
        ta_issued_certs() == number
    })
}

pub fn ta_issued_certs() -> usize {
    let ta = ca_details(&ta_handle());
    ta.published_objects().len() - 2
}

pub fn ta_issued_resources(child: &Handle) -> ResourceSet {
    let ta = ca_details(&ta_handle());
    let child = ta.children().get(child).unwrap();
    if let Some(resources) = child.resources().get(&ResourceClassName::default()) {
        if let Some(cert) = resources.certs_iter().next() {
            return cert.resource_set().clone(); // for our testing the first will do
        }
    }
    ResourceSet::default()
}

pub fn ca_current_resources(handle: &Handle) -> ResourceSet {
    let ca = ca_details(handle);

    if let Some(parent) = ca.parent(&ta_handle()) {
        if let Some(rc) = parent.resources().get(&ResourceClassName::default()) {
            match rc.keys() {
                ResourceClassKeysInfo::Active(current)
                | ResourceClassKeysInfo::RollPending(_, current)
                | ResourceClassKeysInfo::RollNew(_, current)
                | ResourceClassKeysInfo::RollOld(current, _) => {
                    return current.incoming_cert().resources().clone()
                }
                _ => {}
            }
        }
    }

    ResourceSet::default()
}

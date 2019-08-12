extern crate krill_client;
extern crate krill_commons;
extern crate krill_daemon;
extern crate krill_pubc;

use krill_client::options::{CaCommand, Command, TrustAnchorCommand};
use krill_client::report::ApiResponse;
use krill_commons::api::admin::{
    AddChildRequest, AddParentRequest, CertAuthInit, CertAuthPubMode, ChildAuthRequest, Handle,
    ParentCaContact, Token, UpdateChildRequest,
};
use krill_commons::api::ca::{CaParentsInfo, CertAuthInfo, ResourceClassKeysInfo, ResourceSet};
use krill_commons::remote::rfc8183;
use krill_daemon::ca::ta_handle;
use krill_daemon::test::{krill_admin, test_with_krill_server, wait_seconds};

fn init_ta() {
    krill_admin(Command::TrustAnchor(TrustAnchorCommand::Init));
}

fn init_child(handle: &Handle, token: &Token) {
    let init = CertAuthInit::new(handle.clone(), token.clone(), CertAuthPubMode::Embedded);
    krill_admin(Command::CertAuth(CaCommand::Init(init)));
}

fn child_request(handle: &Handle) -> rfc8183::ChildRequest {
    match krill_admin(Command::CertAuth(CaCommand::ChildRequest(handle.clone()))) {
        ApiResponse::Rfc8183ChildRequest(req) => req,
        _ => panic!("Expected child request"),
    }
}

fn add_child_to_ta_embedded(
    handle: &Handle,
    token: &Token,
    resources: ResourceSet,
) -> ParentCaContact {
    let auth = ChildAuthRequest::Embedded(token.clone());
    let req = AddChildRequest::new(handle.clone(), resources, auth);
    let res = krill_admin(Command::TrustAnchor(TrustAnchorCommand::AddChild(req)));

    match res {
        ApiResponse::ParentCaInfo(info) => info,
        _ => panic!("Expected ParentCaInfo response"),
    }
}

fn add_child_to_ta_rfc6492(
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

fn update_child(handle: &Handle, resources: &ResourceSet) {
    let req = UpdateChildRequest::new(None, None, Some(resources.clone()));
    match krill_admin(Command::TrustAnchor(TrustAnchorCommand::UpdateChild(
        handle.clone(),
        req,
    ))) {
        ApiResponse::Empty => {}
        _ => panic!("Expected empty ok response"),
    }
}

fn add_parent_to_ca(handle: &Handle, parent: AddParentRequest) {
    krill_admin(Command::CertAuth(CaCommand::AddParent(
        handle.clone(),
        parent,
    )));
}

fn ca_roll_init(handle: &Handle) {
    krill_admin(Command::CertAuth(CaCommand::KeyRollInit(handle.clone())));
}

fn ca_roll_activate(handle: &Handle) {
    krill_admin(Command::CertAuth(CaCommand::KeyRollActivate(
        handle.clone(),
    )));
}

fn ca_details(handle: &Handle) -> CertAuthInfo {
    match krill_admin(Command::CertAuth(CaCommand::Show(handle.clone()))) {
        ApiResponse::CertAuthInfo(inf) => inf,
        _ => panic!("Expected cert auth info"),
    }
}

fn wait_for<O>(tries: u64, error_msg: &'static str, op: O)
where
    O: Copy + FnOnce() -> bool,
{
    for _counter in 1..tries + 1 {
        if op() == true {
            return;
        }
        wait_seconds(1);
    }
    panic!(error_msg);
}

fn wait_for_resources_on_current_key(handle: &Handle, resources: &ResourceSet) {
    wait_for(
        30,
        "cms child did not get its resource certificate",
        move || {
            let cms_ca_info = ca_details(handle);

            if let CaParentsInfo::Parents(parents) = cms_ca_info.parents() {
                if let Some(parent) = parents.get(&ta_handle()) {
                    if let Some(rc) = parent.resources().get("all") {
                        if let Some(current_resources) = rc.current_resources() {
                            if resources == current_resources {
                                return true;
                            }
                        }
                    }
                }
            }
            false
        },
    )
}

fn wait_for_new_key(handle: &Handle) {
    wait_for(30, "No new key received", move || {
        let cms_ca_info = ca_details(handle);

        if let CaParentsInfo::Parents(parents) = cms_ca_info.parents() {
            if let Some(parent) = parents.get(&ta_handle()) {
                if let Some(rc) = parent.resources().get("all") {
                    match rc.keys() {
                        ResourceClassKeysInfo::RollNew(new, _) => {
                            return new.current_set().number() == 2
                        }
                        _ => return false,
                    }
                }
            }
        }
        false
    })
}

fn wait_for_key_roll_complete(handle: &Handle) {
    wait_for(30, "Key roll did not complete", || {
        let cms_ca_info = ca_details(handle);

        if let CaParentsInfo::Parents(parents) = cms_ca_info.parents() {
            if let Some(parent) = parents.get(&ta_handle()) {
                if let Some(rc) = parent.resources().get("all") {
                    match rc.keys() {
                        ResourceClassKeysInfo::Active(_) => return true,
                        _ => return false,
                    }
                }
            }
        }
        false
    })
}

#[test]
fn ca_under_ta() {
    test_with_krill_server(|_d| {
        let ta_handle = ta_handle();
        init_ta();

        // Embedded CA ----------------------------------------------------------------------------

        let emb_child_handle = Handle::from("child");
        let emb_child_token = Token::from("child");
        let emb_child_resources = ResourceSet::from_strs("", "192.168.0.0/16", "").unwrap();

        init_child(&emb_child_handle, &emb_child_token);

        let parent = {
            let parent_contact =
                add_child_to_ta_embedded(&emb_child_handle, &emb_child_token, emb_child_resources);
            AddParentRequest::new(ta_handle.clone(), parent_contact)
        };

        add_parent_to_ca(&emb_child_handle, parent);

        // RFC6492 CA -----------------------------------------------------------------------------

        let cms_child_handle = Handle::from("rfc6492");
        let cms_child_token = Token::from("rfc6492");
        let cms_child_resources = ResourceSet::from_strs("", "10.0.0.0/16", "").unwrap();

        init_child(&cms_child_handle, &cms_child_token);
        let req = child_request(&cms_child_handle);

        let parent = {
            let contact =
                add_child_to_ta_rfc6492(&cms_child_handle, req, cms_child_resources.clone());
            AddParentRequest::new(ta_handle.clone(), contact)
        };

        add_parent_to_ca(&cms_child_handle, parent);
        wait_for_resources_on_current_key(&cms_child_handle, &cms_child_resources);

        let cms_child_resources = ResourceSet::from_strs("AS65000", "10.0.0.0/16", "").unwrap();
        update_child(&cms_child_handle, &cms_child_resources);
        wait_for_resources_on_current_key(&cms_child_handle, &cms_child_resources);

        ca_roll_init(&cms_child_handle);
        wait_for_new_key(&cms_child_handle);

        ca_roll_activate(&cms_child_handle);
        wait_for_key_roll_complete(&cms_child_handle);
    });
}

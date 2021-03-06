extern crate krill;
extern crate pretty;
extern crate rpki;

use std::fs;
use std::str::FromStr;

use rpki::manifest::Manifest;
use rpki::roa::Roa;

use krill::cli::options::{CaCommand, Command, PublishersCommand};
use krill::cli::report::ApiResponse;
use krill::commons::api::{
    CaRepoDetails, CurrentRepoState, Handle, ParentCaReq, PublisherDetails, PublisherHandle,
    RepositoryUpdate, ResourceSet, RoaDefinition, RoaDefinitionUpdates,
};
use krill::commons::remote::rfc8183;
use krill::daemon::ca::ta_handle;
use krill::daemon::test::{
    add_child_to_ta_embedded, add_parent_to_ca, ca_route_authorizations_update,
    init_child_with_embedded_repo, krill_admin, krill_pubd_admin, start_krill_pubd_server,
    test_with_krill_server, wait_for, wait_for_current_resources, PubdTestContext,
};

fn repository_response(
    publisher: &PublisherHandle,
    server: PubdTestContext,
) -> rfc8183::RepositoryResponse {
    let command = Command::Publishers(PublishersCommand::RepositoryResponse(publisher.clone()));
    match krill_pubd_admin(command, server) {
        ApiResponse::Rfc8183RepositoryResponse(response) => response,
        _ => panic!("Expected repository response."),
    }
}

fn add_publisher(req: rfc8183::PublisherRequest, server: PubdTestContext) {
    let command = Command::Publishers(PublishersCommand::AddPublisher(req));
    krill_pubd_admin(command, server);
}

fn details_publisher(publisher: &PublisherHandle, server: PubdTestContext) -> PublisherDetails {
    let command = Command::Publishers(PublishersCommand::ShowPublisher(publisher.clone()));
    let res = krill_pubd_admin(command, server);
    match res {
        ApiResponse::PublisherDetails(details) => details,
        _ => panic!("Expected publisher details"),
    }
}

fn repo_details(ca: &Handle) -> CaRepoDetails {
    let command = Command::CertAuth(CaCommand::RepoDetails(ca.clone()));
    match krill_admin(command) {
        ApiResponse::RepoDetails(details) => details,
        _ => panic!("Expected repo details"),
    }
}

fn repo_state(ca: &Handle) -> CurrentRepoState {
    let command = Command::CertAuth(CaCommand::RepoState(ca.clone()));
    match krill_admin(command) {
        ApiResponse::RepoState(state) => state,
        _ => panic!("Expected repo state"),
    }
}

fn repo_update(ca: &Handle, update: RepositoryUpdate) {
    let command = Command::CertAuth(CaCommand::RepoUpdate(ca.clone(), update));
    krill_admin(command);
}

fn publisher_request(ca: &Handle) -> rfc8183::PublisherRequest {
    let command = Command::CertAuth(CaCommand::RepoPublisherRequest(ca.clone()));
    match krill_admin(command) {
        ApiResponse::Rfc8183PublisherRequest(req) => req,
        _ => panic!("Expected publisher request"),
    }
}

/// This tests that you can run krill with an embedded TA and CA, and
/// have the CA publish at another krill instance which is is set up
/// as a publication server only (i.e. it just has no TA and CAs).
#[test]
fn remote_publication() {
    test_with_krill_server(|_d| {
        let pubd_dir = start_krill_pubd_server();

        let ta_handle = ta_handle();

        let child = Handle::from_str_unsafe("child");

        // Set up child as a child of the TA
        {
            init_child_with_embedded_repo(&child);
            let child_resources = ResourceSet::from_strs("", "10.0.0.0/16", "").unwrap();

            let parent = {
                let parent_contact = add_child_to_ta_embedded(&child, child_resources.clone());
                ParentCaReq::new(ta_handle, parent_contact)
            };

            add_parent_to_ca(&child, parent);
            wait_for_current_resources(&child, &child_resources);
        }

        // Child should now publish using the embedded repo
        wait_for(30, "Should see objects at embedded location", || {
            let child_repo_details = repo_details(&child);
            assert!(child_repo_details.contact().is_embedded());

            let state = repo_state(&child);
            let list = state.as_list();
            list.elements().len() == 2
        });

        // Add some roas to have more to migrate when moving publication servers
        let route_1 = RoaDefinition::from_str("10.0.0.0/24 => 64496").unwrap();
        let route_2 = RoaDefinition::from_str("10.0.2.0/23 => 64496").unwrap();
        let mut updates = RoaDefinitionUpdates::empty();
        updates.add(route_1);
        updates.add(route_2);
        ca_route_authorizations_update(&child, updates);

        // Add child to the secondary publication server
        let publisher_request = publisher_request(&child);
        add_publisher(publisher_request, PubdTestContext::Secondary);

        // The child should now be known at the pub server and have no files
        let details_at_pubd = details_publisher(&child, PubdTestContext::Secondary);
        assert_eq!(details_at_pubd.current_files().len(), 0);

        // Get a Repository Response for the child CA
        let response = repository_response(&child, PubdTestContext::Secondary);

        // Update the repo for the child
        let update = RepositoryUpdate::Rfc8181(response);
        repo_update(&child, update);

        // Child should now publish using the remote repo
        wait_for(30, "Should see objects at new location", || {
            let child_repo_details = repo_details(&child);
            assert!(child_repo_details.contact().is_rfc8183());
            let state = repo_state(&child);
            let list = state.as_list();

            list.elements().len() == 4
        });

        {
            // Test that the new repo URI is used in newly published objects

            let details = details_publisher(&child, PubdTestContext::Secondary);

            let mft = details
                .current_files()
                .iter()
                .find(|e| e.uri().ends_with(".mft"))
                .unwrap();
            let mft = Manifest::decode(mft.base64().to_bytes(), true).unwrap();
            let mft_uri = mft.cert().signed_object().unwrap();
            let crl_uri = mft.cert().crl_uri().unwrap();
            assert!(mft_uri.to_string().starts_with("rsync://remotehost/repo/"));
            assert!(crl_uri.to_string().starts_with("rsync://remotehost/repo/"));

            for roa in details
                .current_files()
                .iter()
                .filter(|e| e.uri().ends_with(".roa"))
            {
                let roa = Roa::decode(roa.base64().to_bytes(), true).unwrap();
                let roa_uri = roa.cert().signed_object().unwrap();
                let crl_uri = roa.cert().crl_uri().unwrap();
                assert!(roa_uri.to_string().starts_with("rsync://remotehost/repo/"));
                assert!(crl_uri.to_string().starts_with("rsync://remotehost/repo/"));
            }
        }

        // Child should now clean up the old repo
        wait_for(10, "Child should clean up at old repository", || {
            let details_at_main = details_publisher(&child, PubdTestContext::Main);
            details_at_main.current_files().is_empty()
        });

        // Now let's migrate back, so that we see that works too.

        // Get a Repository Response for the child CA
        let response = repository_response(&child, PubdTestContext::Main);

        // Update the repo for the child
        let update = RepositoryUpdate::Rfc8181(response);
        repo_update(&child, update);

        // Child should now publish using the main repo
        wait_for(30, "Should see objects at new location", || {
            let child_repo_details = repo_details(&child);
            assert!(child_repo_details.contact().is_rfc8183());
            let state = repo_state(&child);
            let list = state.as_list();
            list.elements().len() == 4
        });

        // Child should now clean up the old repo
        wait_for(10, "Child should clean up at old repository", || {
            let details_at_main = details_publisher(&child, PubdTestContext::Secondary);
            details_at_main.current_files().is_empty()
        });

        fs::remove_dir_all(pubd_dir).unwrap();
    });
}

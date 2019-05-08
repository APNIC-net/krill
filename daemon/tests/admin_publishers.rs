extern crate krill_commons;
extern crate krill_client;
extern crate krill_daemon;

use krill_client::options::{AddPublisher, Command, Options, PublishersCommand};
use krill_client::KrillClient;
use krill_client::report::ApiResponse;
use krill_client::report::ReportFormat;
use krill_commons::util::test;


fn execute_krillc_command(command: Command) -> ApiResponse {
    let krillc_opts = Options::new(
        test::https_uri("https://localhost:3000/"),
        "secret",
        ReportFormat::Json,
        command
    );
    match KrillClient::test(krillc_opts) {
        Ok(res) => res, // ok
        Err(e) => {
            panic!("{}", e)
        }
    }
}

fn add_publisher(handle: &str, base_uri: &str, token: &str) {
    let command = Command::Publishers(PublishersCommand::Add(
        AddPublisher {
            handle: handle.to_string(),
            base_uri: test::rsync_uri(base_uri),
            token: token.to_string()
        }
    ));
    execute_krillc_command(command);
}

fn deactivate_publisher(handle: &str) {
    let command = Command::Publishers(
        PublishersCommand::Deactivate(handle.to_string())
    );

    execute_krillc_command(command);
}

fn list_publishers() -> ApiResponse {
    let command = Command::Publishers(
        PublishersCommand::List
    );

    execute_krillc_command(command)
}

fn details_publisher(handle: &str) -> ApiResponse {
    let command = Command::Publishers(
        PublishersCommand::Details(handle.to_string())
    );

    execute_krillc_command(command)
}

#[test]
fn admin_publishers() {
    krill_daemon::test::test_with_krill_server(|_d| {

        let handle = "alice";
        let token = "secret";
        let base_rsync_uri_alice = "rsync://localhost/repo/alice/";

        // Add client "alice"
        add_publisher(handle, base_rsync_uri_alice, token);

        // Find "alice" in list
        let res = list_publishers();
        match res {
            ApiResponse::PublisherList(list) => {
                // there should be one and it should be alice
                assert_eq!(1, list.publishers().len());
                let alice = &list.publishers().get(0).unwrap();
                assert_eq!("alice", alice.id());
            },
            _ => panic!("Expected publisher list")
        }

        // Find details for alice
        let details_res = details_publisher("alice");
        match details_res {
            ApiResponse::PublisherDetails(details) => {
                assert_eq!("alice", details.handle());
                assert_eq!(false, details.deactivated());
            },
            _ => panic!("Expected details")
        }

        // Remove alice
        deactivate_publisher(handle);

        // Expect that alice still exists, but is now deactivated.
        let details_res = details_publisher("alice");
        match details_res {
            ApiResponse::PublisherDetails(details) => {
                assert_eq!("alice", details.handle());
                assert_eq!(true, details.deactivated());
            },
            _ => panic!("Expected details")
        }

    });
}


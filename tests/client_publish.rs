extern crate actix;
extern crate futures;
extern crate reqwest;
extern crate rpki;
extern crate krill;
extern crate krill_commons;
extern crate serde_json;
extern crate tokio;
extern crate bytes;
extern crate toml;
extern crate krill_cms_proxy;

use std::{thread, time};
use std::collections::HashSet;
use std::path::PathBuf;
use actix::System;
use krill::krillc::options::{AddPublisher, Command, Options, PublishersCommand, Rfc8181Command, AddRfc8181Client};
use krill::krillc::KrillClient;
use krill::krilld::config::Config;
use krill::krilld::http::server::PubServerApp;
use krill::pubc::apiclient;
use krill::pubc::apiclient::ApiResponse;
use krill::krillc::report::ReportFormat;
use krill_commons::util::file::CurrentFile;
use krill_commons::util::file;
use krill_commons::util::httpclient;
use krill_commons::util::test;
use krill::pubc::cmsclient::PubClient;
use krill_cms_proxy::rfc8183::RepositoryResponse;

fn list(server_uri: &str, handle: &str, token: &str) -> apiclient::Options {
    let conn = apiclient::Connection::build(server_uri, handle, token).unwrap();
    let cmd = apiclient::Command::List;
    let fmt = apiclient::Format::Json;

    apiclient::Options::new(conn, cmd, fmt)
}

fn sync(
    server_uri: &str,
    handle: &str,
    token: &str,
    syncdir: &PathBuf,
    base_uri: &str
) -> apiclient::Options {
    let conn = apiclient::Connection::build(server_uri, handle, token).unwrap();
    let cmd = apiclient::Command::sync(syncdir.to_str().unwrap(), base_uri).unwrap();
    let fmt = apiclient::Format::Json;

    apiclient::Options::new(conn, cmd, fmt)
}

fn execute_krillc_command(command: Command) {
    let krillc_opts = Options::new(
        test::http_uri("http://localhost:3000/"),
        "secret",
        ReportFormat::Default,
        command
    );
    match KrillClient::process(krillc_opts) {
        Ok(_res) => {}, // ok
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

fn remove_publisher(handle: &str) {
    let command = Command::Publishers(
        PublishersCommand::Deactivate(handle.to_string())
    );

    execute_krillc_command(command);
}

fn add_rfc8181_client(handle: &str, token: &str, base_work_dir: &PathBuf) -> PubClient {
    let client_dir = test::create_sub_dir(base_work_dir);
    let mut client = PubClient::build(&client_dir).unwrap();
    client.init(handle).unwrap();
    let pr = client.publisher_request().unwrap();
    let mut pr_path = base_work_dir.clone();
    pr_path.push(&format!("{}.xml", handle));
    pr.save(&pr_path).unwrap();

    let command = Command::Rfc8181(
        Rfc8181Command::Add(
            AddRfc8181Client { token: token.to_string(), xml: pr_path }
        )
    );

    execute_krillc_command(command);

    client
}

fn get_repository_response(handle: &str) -> RepositoryResponse {
    let uri = format!("http://localhost:3000/api/v1/rfc8181/{}/response.xml", handle);
    let content_type = "application/xml";
    let token = Some("secret");

    let xml = httpclient::get_text(
        &uri, content_type, token
    ).unwrap();

    RepositoryResponse::decode(xml.as_bytes()).unwrap()
}

#[test]
fn client_publish_through_api() {
    test::test_with_tmp_dir(|d| {

        let server_uri = "http://localhost:3000/";
        let handle = "alice";
        let token = "secret";
        let base_rsync_uri_alice = "rsync://127.0.0.1/repo/alice/";
        let base_rsync_uri_bob = "rsync://127.0.0.1/repo/bob/";

        // Set up a test PubServer Config
        let server_conf = {
            // Use a data dir for the storage
            let data_dir = test::create_sub_dir(&d);
            Config::test(&data_dir)
        };

        // Start the server
        thread::spawn(||{
            System::run(move || {
                PubServerApp::start(&server_conf);
            })
        });

        // XXX TODO: Find a better way to know the server is ready!
        thread::sleep(time::Duration::from_millis(500));

        // Add client "alice"
        add_publisher(handle, base_rsync_uri_alice, token);

        // Calls to api should require the correct token
        {
            let res = apiclient::execute(list(
                server_uri,
                handle,
                "wrong token"
            ));

            match res {
                Err(apiclient::Error::HttpClientError
                    (httpclient::Error::Forbidden)) => {},
                Err(e) => panic!("Expected forbidden, got: {}", e),
                _ => panic!("Expected forbidden")
            }
        }

        // List files at server, expect no files
        {
            let list = apiclient::execute(list(
                server_uri,
                handle,
                token
            )).unwrap();

            match list {
                ApiResponse::List(list) => {
                    assert_eq!(0, list.elements().len());
                },
                _ => panic!("Expected list")
            }
        }


        // Create files on disk to sync
        let sync_dir = test::create_sub_dir(&d);
        let file_a = CurrentFile::new(
            test::rsync_uri("rsync://127.0.0.1/repo/alice/a.txt"),
            &test::as_bytes("a")
        );
        let file_b = CurrentFile::new(
            test::rsync_uri("rsync://127.0.0.1/repo/alice/b.txt"),
            &test::as_bytes("b")
        );
        let file_c = CurrentFile::new(
            test::rsync_uri("rsync://127.0.0.1/repo/alice/c.txt"),
            &test::as_bytes("c")
        );

        file::save_in_dir(&file_a.to_bytes(), &sync_dir, "a.txt").unwrap();
        file::save_in_dir(&file_b.to_bytes(), &sync_dir, "b.txt").unwrap();
        file::save_in_dir(&file_c.to_bytes(), &sync_dir, "c.txt").unwrap();


        // Must refuse syncing files outside of publisher base dir
        {
            let api_res = apiclient::execute(sync(
                server_uri,
                handle,
                token,
                &sync_dir,
                base_rsync_uri_bob
            ));

            assert!(api_res.is_err())
        }


        // Sync files
        {
            let api_res = apiclient::execute(sync(
                server_uri,
                handle,
                token,
                &sync_dir,
                base_rsync_uri_alice
            )).unwrap();

            assert_eq!(ApiResponse::Success, api_res);
        }

        // We should now see these files when we list
        {
            let list = apiclient::execute(list(
                server_uri,
                handle,
                token
            )).unwrap();

            match list {
                ApiResponse::List(list) => {
                    assert_eq!(3, list.elements().len());

                    let returned_set: HashSet<_>  = list.elements().into_iter().collect();

                    let list_el_a = file_a.into_list_element();
                    let list_el_b = file_b.into_list_element();
                    let list_el_c = file_c.clone().into_list_element();

                    let expected_elements = vec![
                        &list_el_a,
                        &list_el_b,
                        &list_el_c
                    ];
                    let expected_set: HashSet<_> = expected_elements.into_iter().collect();
                    assert_eq!(expected_set, returned_set);
                },
                _ => panic!("Expected list")
            }
        }

        // XXX TODO We should also see these files in RRDP


        // Now we should be able to delete it all again
        file::delete_in_dir(&sync_dir, "a.txt").unwrap();
        file::delete_in_dir(&sync_dir, "b.txt").unwrap();

        // Sync files
        {
            let api_res = apiclient::execute(sync(
                server_uri,
                handle,
                token,
                &sync_dir,
                base_rsync_uri_alice
            )).unwrap();

            assert_eq!(ApiResponse::Success, api_res);
        }

        // List files at server, expect 1 file (c.txt)
        {
            let list = apiclient::execute(list(
                server_uri,
                handle,
                token
            )).unwrap();

            match list {
                ApiResponse::List(list) => {
                    assert_eq!(1, list.elements().len());

                    let returned_set: HashSet<_>  = list.elements().into_iter().collect();

                    let list_el_c = file_c.into_list_element();

                    let expected_elements = vec![&list_el_c];
                    let expected_set: HashSet<_> = expected_elements.into_iter().collect();
                    assert_eq!(expected_set, returned_set);
                },
                _ => panic!("Expected list")
            }
        }

        // Remove alice
        remove_publisher(handle);

        // XXX TODO Must remove files when removing publisher
        // Expect that c.txt is removed when looking at latest snapshot.
        file::delete_in_dir(&sync_dir, "c.txt").unwrap();
    });
}

#[test]
pub fn client_publish_through_cms() {
    test::test_with_tmp_dir(|d| {
//        let server_uri = "http://localhost:3000/";
        let handle = "alice";
        let token = "secret";
        let base_rsync_uri_alice = "rsync://127.0.0.1/repo/alice/";

        // Set up a test PubServer Config
        let server_conf = {
            // Use a data dir for the storage
            let data_dir = test::create_sub_dir(&d);
            Config::test(&data_dir)
        };

        // Start the server
        thread::spawn(|| {
            System::run(move || {
                PubServerApp::start(&server_conf);
            })
        });

        // XXX TODO: Find a better way to know the server is ready!
        thread::sleep(time::Duration::from_millis(500));

        // Add client "alice"
        add_publisher(handle, base_rsync_uri_alice, token);

        // Add RFC8181 client for alice
        let mut client = add_rfc8181_client(handle, token, &d);

        let response = get_repository_response(handle);

        client.process_repo_response(&response).unwrap();
    });

}
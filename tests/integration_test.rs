extern crate futures;
extern crate hyper;
extern crate rpki;
extern crate rpubd;
extern crate serde_json;
extern crate tokio;

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use hyper::Client;
use rpki::oob::exchange::PublisherRequest;
use rpubd::test;
use rpubd::pubd::config::Config;
use rpubd::pubd::server;
use rpubd::provisioning::publisher::Publisher;

fn save_pr(base_dir: &str, file_name: &str, pr: &PublisherRequest) {
    let full_name = PathBuf::from(format!("{}/{}", base_dir, file_name));
    let mut f = File::create(full_name).unwrap();
    let xml = pr.encode_vec();
    f.write(xml.as_ref()).unwrap();
}


#[test]
fn testing() {

    use std::str;
    use std::{thread, time};
    use tokio::prelude::*;
    use tokio::runtime::Runtime;


    test::test_with_tmp_dir(|d| {

        // Use a data dir for the storage
        let data_dir = test::create_sub_dir(&d);

        // Start with an xml dir with two PRs for alice and bob
        let xml_dir = test::create_sub_dir(&d);
        let pr_alice = test::new_publisher_request("alice");
        let pr_bob = test::new_publisher_request("bob");
        save_pr(&xml_dir, "alice.xml", &pr_alice);
        save_pr(&xml_dir, "bob.xml", &pr_bob);

        let config = Config::test(data_dir, xml_dir);

        // Start the server
        let mut rt = Runtime::new().unwrap();
        rt.spawn(
            future::lazy(move || {
                server::serve(&config);
                Ok(())
            })
        );

        // XXX TODO: There must be a better way to know that the server is
        // ready!
        thread::sleep(time::Duration::from_secs(1));

        let url = "http://localhost:3000/publishers".parse().unwrap();
        let client = Client::new();

        let fut = client
            .get(url)
            .and_then(|res| {
                res.into_body().concat2()
            })
            .and_then(|body| {
                let pl: Vec<Publisher> = serde_json::from_str(
                        str::from_utf8(&body).unwrap()
                ).unwrap();
                assert_eq!(2, pl.len());
                Ok(())
            })
            .map_err(|e| {
                println!("{}", e);
            });


        rt.block_on(fut).unwrap();



        rt.shutdown_now();
    });
}


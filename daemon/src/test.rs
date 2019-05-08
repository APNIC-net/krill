//! Support for tests in other modules using a running krill server

use std::{thread, time};
use std::path::PathBuf;

use actix::System;

use krill_commons::util::test;
use krill_client::KrillClient;
use krill_client::options::{Command, Options};
use krill_client::report::{ApiResponse, ReportFormat};
use crate::config::Config;
use crate::http::server::PubServerApp;


pub fn test_with_krill_server<F>(op: F) where F: FnOnce(PathBuf) -> () {
    test::test_with_tmp_dir(|dir| {
        // Set up a test PubServer Config
        let server_conf = {
            // Use a data dir for the storage
            let data_dir = test::create_sub_dir(&dir);
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

        op(dir)
    })
}


pub fn execute_krillc_command(command: Command) -> ApiResponse {
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
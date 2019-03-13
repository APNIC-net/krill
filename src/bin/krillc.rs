extern crate krill;
extern crate krill_commons;

use krill_commons::api::publishers::ReportFormat;
use krill::krillc::KrillClient;
use krill::krillc::options::Options;

fn main() {
    match Options::from_args() {
        Ok(options) => {
            let format = options.format().clone();
            match KrillClient::report(options) {
                Ok(()) => {} //,
                Err(e) => {
                    if format != ReportFormat::None {
                        eprintln!("{}", e);
                    }
                    ::std::process::exit(1);
                }
            }
        },
        Err(e) => {
            eprintln!("{}", e);
            ::std::process::exit(1);
        }
    }
}
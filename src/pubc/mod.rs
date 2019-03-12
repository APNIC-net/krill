pub mod apiclient;

use std::path::PathBuf;
use rpki::uri;
use crate::api::publication_data;
use crate::util::file;

pub fn create_delta(
    list_reply: &publication_data::ListReply,
    dir: &PathBuf,
    base_rsync: &uri::Rsync
) -> Result<publication_data::PublishDelta, Error> {
    let mut delta_builder = publication_data::PublishDeltaBuilder::new();

    let current = file::crawl_incl_rsync_base(dir, base_rsync)?;

    // loop through what the server has and find the ones to withdraw
    for p in list_reply.elements() {
        if current.iter().find(|c| c.uri() == p.uri()).is_none() {
            delta_builder.add_withdraw(
                publication_data::Withdraw::from_list_element(p)
            );
        }
    }

    // loop through all current files on disk and find out which ones need
    // to be added to, which need to be updated at, or for which no change is
    // needed at the server.
    for f in current {
        match list_reply.elements().iter().find(|pbl| pbl.uri() == f.uri()) {
            None => delta_builder.add_publish(f.as_publish()),
            Some(pbl) => {
                if pbl.hash() != f.hash() {
                    delta_builder.add_update(f.as_update(pbl.hash()))
                }
            }
        }
    }

    Ok(delta_builder.finish())
}


//------------ Error ---------------------------------------------------------

#[derive(Debug, Display)]
pub enum Error {
    #[display(fmt = "{}", _0)]
    RecursorError(file::RecursorError),
}

impl From<file::RecursorError> for Error {
    fn from(e: file::RecursorError) -> Self {
        Error::RecursorError(e)
    }
}
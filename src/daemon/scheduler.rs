//! Deal with asynchronous scheduled processes, either triggered by an
//! event that occurred, or planned (e.g. re-publishing).

use std::sync::Arc;
use std::time::Duration;

use clokwerk::{self, ScheduleHandle, TimeUnits};

use crate::commons::util::softsigner::OpenSslSigner;
use crate::daemon::ca::CaServer;
use crate::daemon::mq::{EventQueueListener, QueueEvent};
use crate::pubd::PubServer;

pub struct Scheduler {
    /// Responsible for listening to events and executing triggered processes, such
    /// as publication of newly generated RPKI objects.
    #[allow(dead_code)] // just need to keep this in scope
    event_sh: ScheduleHandle,

    /// Responsible for periodically republishing so that MFTs and CRLs do not go stale.
    #[allow(dead_code)] // just need to keep this in scope
    republish_sh: ScheduleHandle,

    /// Responsible for letting CA check with their parents whether their resource
    /// entitlements have changed *and* for the shrinking of issued certificates, if
    /// they are not renewed within the configured grace period.
    #[allow(dead_code)] // just need to keep this in scope
    ca_refresh_sh: ScheduleHandle,
}

impl Scheduler {
    pub fn build(
        event_queue: Arc<EventQueueListener>,
        caserver: Arc<CaServer<OpenSslSigner>>,
        pubserver: Arc<PubServer>,
        ca_refresh_rate: u32,
    ) -> Self {
        let event_sh = make_event_sh(event_queue, caserver.clone(), pubserver);
        let republish_sh = make_republish_sh(caserver.clone());
        let ca_refresh_sh = make_ca_refresh_sh(caserver, ca_refresh_rate);

        Scheduler {
            event_sh,
            republish_sh,
            ca_refresh_sh,
        }
    }
}

fn make_event_sh(
    event_queue: Arc<EventQueueListener>,
    caserver: Arc<CaServer<OpenSslSigner>>,
    pubserver: Arc<PubServer>,
) -> ScheduleHandle {
    let mut scheduler = clokwerk::Scheduler::new();
    scheduler.every(1.seconds()).run(move || {
        while let Some(evt) = event_queue.pop() {
            match evt {
                QueueEvent::Delta(handle, version, delta) => {
                    trace!("Trigger publication for '{}' version '{}'", handle, version);
                    if let Err(e) = pubserver.publish(&handle, delta) {
                        error!("Failed to publish for CA: {}, error: {}", handle, e);
                    }
                }
                QueueEvent::ResourceClassRemoved(handle, _, parent, revocations) => {
                    trace!(
                        "Trigger send revoke requests for removed RC for '{}' under '{}'",
                        handle,
                        parent
                    );
                    if caserver
                        .send_revoke_requests(&handle, &parent, revocations)
                        .is_err()
                    {
                        debug!("Could not revoke key for removed resource class. This is not \
                        an issue, because typically the parent will revoke our keys pro-actively, \
                        just before removing the resource class entitlements.");
                    }
                }
                QueueEvent::ParentAdded(handle, _, parent) => {
                    trace!(
                        "Get updates for '{}' from added parent '{}'.",
                        handle,
                        parent
                    );
                    if let Err(e) = caserver.get_updates_from_parent(&handle, &parent) {
                        error!(
                            "Error getting updates for '{}', from parent '{}',  error: '{}'",
                            &handle, &parent, e
                        )
                    }
                }
                QueueEvent::RequestsPending(handle, _) => {
                    trace!("Get updates for pending requests for '{}'.", handle);
                    if let Err(e) = caserver.send_all_requests(&handle) {
                        error!("Sending pending requests for '{}', error '{}'", &handle, e);
                    }
                }
            }
        }
    });
    scheduler.watch_thread(Duration::from_millis(100))
}

fn make_republish_sh(caserver: Arc<CaServer<OpenSslSigner>>) -> ScheduleHandle {
    let mut scheduler = clokwerk::Scheduler::new();
    scheduler.every(1.hours()).run(move || {
        info!("Triggering background republication for all CAs");
        if let Err(e) = caserver.republish_all() {
            error!("Background republishing failed: {}", e);
        }
    });
    scheduler.watch_thread(Duration::from_millis(100))
}

fn make_ca_refresh_sh(caserver: Arc<CaServer<OpenSslSigner>>, refresh_rate: u32) -> ScheduleHandle {
    let mut scheduler = clokwerk::Scheduler::new();
    scheduler.every(refresh_rate.seconds()).run(move || {
        info!("Triggering background refresh for all CAs");
        caserver.refresh_all()
    });
    scheduler.watch_thread(Duration::from_millis(100))
}

//! A simple message queue, responsible for listening for (CA) events,
//! making them available for triggered processing, such as publishing
//! signed material, or asking a newly added parent for resource
//! entitlements.

use std::collections::VecDeque;
use std::fmt;
use std::sync::RwLock;

use krill_commons::api::admin::{
    Handle,
    ParentCaContact
};
use krill_commons::api::ca::PublicationDelta;
use krill_commons::eventsourcing;

use crate::ca::{
    Signer,
    Evt,
    EvtDet,
    CertAuth,
    ParentHandle
};

//------------ QueueEvent ----------------------------------------------------

/// This type contains all the events of interest for a KrillServer, with
/// the details needed for triggered processing.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum QueueEvent {
    ParentAdded(Handle, ParentHandle, ParentCaContact),
    Delta(Handle, PublicationDelta),
}

#[derive(Debug)]
pub struct EventQueueListener {
    q: RwLock<Box<EventQueueStore>>
}

impl EventQueueListener {
    pub fn in_mem() -> Self {
        EventQueueListener { q: RwLock::new(Box::new(MemoryEventQueue::new()))}
    }
}

impl EventQueueListener {
    pub fn pop(&self) -> Option<QueueEvent> {
        self.q.write().unwrap().pop()
    }

    fn push_back(&self, evt: QueueEvent) {
        self.q.write().unwrap().push_back(evt)
    }
}

// TODO: Is this unsafe here? I would think the RwLock is safe, but..
unsafe impl Send for EventQueueListener {}
unsafe impl Sync for EventQueueListener {}


/// Implement listening for CertAuth Published events.
impl<S: Signer> eventsourcing::EventListener<CertAuth<S>> for EventQueueListener {
    fn listen(&self, _ca: &CertAuth<S>, event: &Evt) {

        use krill_commons::eventsourcing::Event;

        let json = serde_json::to_string_pretty(&event).unwrap();
        debug!("Seen CertAuth event: {}", json);

        let handle = event.handle();
        match event.details() {
            EvtDet::Published(_, _, _, delta) => {
                let evt = QueueEvent::Delta(handle.clone(), delta.clone());
                self.push_back(evt);
            },
            EvtDet::TaPublished(delta) => {
                let evt = QueueEvent::Delta(handle.clone(), delta.clone());
                self.push_back(evt);
            },
            EvtDet::ParentAdded(parent, contact) => {
                let evt = QueueEvent::ParentAdded(
                    handle.clone(),
                    parent.clone(),
                    contact.clone()
                );
                self.push_back(evt);
            },
            _ => {}
        }
    }
}

//------------ EventQueue ----------------------------------------------------

/// This trait provides the public contract for an EventQueue used by the
/// KrillServer. First implementation can be a simple in memory thing, but
/// we will need someting more robust, and possibly multi-master later.
///
/// The EventQueue should implement Eventlistener
trait EventQueueStore: fmt::Debug {
    fn pop(&self) -> Option<QueueEvent>;
    fn push_back(&self, evt: QueueEvent);
}


//------------ MemoryEventQueue ----------------------------------------------

/// In memory event queue implementation.
#[derive(Debug)]
struct MemoryEventQueue {
    q: RwLock<VecDeque<QueueEvent>>
}

impl MemoryEventQueue {
    pub fn new() -> Self {
        MemoryEventQueue { q: RwLock::new(VecDeque::new())}
    }
}

impl EventQueueStore for MemoryEventQueue {
    fn pop(&self) -> Option<QueueEvent> {
        self.q.write().unwrap().pop_front()
    }

    fn push_back(&self, evt: QueueEvent) {
        self.q.write().unwrap().push_back(evt);
    }
}



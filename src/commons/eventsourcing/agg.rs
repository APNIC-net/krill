use std::fmt;

use super::{Command, Event, Storable};
use commons::eventsourcing::cmd::StoredCommand;

//------------ Aggregate -----------------------------------------------------

/// This trait defines an Aggregate for use with the event sourcing framwork.
///
/// An aggregate is term coming from DDD (Domain Driven Design) and is used to
/// describe an abstraction where a cluster of structs (the aggregate) provides
/// a 'bounded context' for functionality that is exposed only by a single top-level
/// struct: the aggregate root. Here we name this aggregate root simply 'Aggregate'
/// for brevity.
///
/// The aggregate root is responsible for guarding its own consistency. In the
/// context of the event sourcing framework this means that it can be sent a command,
/// through the [`process_command`] method. A command represents an intent to
/// achieve something sent by the used of the aggregate. The Aggregate will then take
/// this intent and decide whether it can be executed. If successful a number of
/// 'events' are returned that contain state changes to the aggregate. These events
/// still need to be applied to become persisted.
pub trait Aggregate: Storable + Send + Sync + 'static {
    type Command: Command<Event = Self::Event>;
    type Event: Event;
    type InitEvent: Event;
    type Error: std::error::Error;

    /// Creates a new instance. Expects an event with data needed to
    /// initialise the instance. Typically this means that a specific
    /// 'create' event is passed, with all the needed data, or just an empty
    /// marker if no data is needed. Implementations must return an error in
    /// case the instance cannot be created.
    fn init(event: Self::InitEvent) -> Result<Self, Self::Error>;

    /// Returns the current version of the aggregate.
    fn version(&self) -> u64;

    /// Applies the event to this. This MUST not result in any errors, and
    /// this MUST be side-effect free. Applying the event just updates the
    /// internal data of the aggregate.
    ///
    /// Note the event is moved. This is done because we want to avoid
    /// doing additional allocations where we can.
    fn apply(&mut self, event: Self::Event);

    /// Applies all events. Assumes that the list ordered, starting with the
    /// oldest event, applicable, self.version matches the oldest event, and
    /// contiguous, i.e. there are no missing events.
    fn apply_all(&mut self, events: Vec<Self::Event>) {
        for event in events {
            self.apply(event);
        }
    }

    /// Processes a command. I.e. validate the command, and return a list of
    /// events that will result in the desired new state, but do not apply
    /// these event here.
    ///
    /// The command is moved, because we want to enable moving its data
    /// without reallocating.
    fn process_command(&self, command: Self::Command) -> Result<Vec<Self::Event>, Self::Error>;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AggregateCommandHistory {
    commands: Vec<StoredCommand>,
}

//------------ AggregateHistory ----------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AggregateHistory<A: Aggregate> {
    init: A::InitEvent,
    events: Vec<A::Event>,
}

impl<A: Aggregate> AggregateHistory<A> {
    pub fn new(init: A::InitEvent, events: Vec<A::Event>) -> Self {
        AggregateHistory { init, events }
    }

    pub fn unpack(self) -> (A::InitEvent, Vec<A::Event>) {
        (self.init, self.events)
    }
}

impl<A: Aggregate> fmt::Display for AggregateHistory<A> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "{}", self.init)?;
        for evt in &self.events {
            writeln!(f, "{}", evt)?;
        }
        Ok(())
    }
}

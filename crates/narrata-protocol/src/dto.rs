use prost::{Enumeration, Message, Oneof};

#[derive(Clone, PartialEq, Message)]
pub struct Request {
    #[prost(uint32, tag = "1")]
    pub protocol_version: u32,
    #[prost(uint64, tag = "2")]
    pub request_id: u64,
    #[prost(
        oneof = "request::Body",
        tags = "10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20"
    )]
    pub body: Option<request::Body>,
}

pub mod request {
    use super::*;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Body {
        #[prost(message, tag = "10")]
        EngineCreate(EngineCreate),
        #[prost(message, tag = "11")]
        ProgramLoad(ProgramLoad),
        #[prost(message, tag = "12")]
        SessionCreate(SessionCreate),
        #[prost(message, tag = "13")]
        SessionLoad(SessionLoad),
        #[prost(message, tag = "14")]
        Dispatch(Dispatch),
        #[prost(message, tag = "15")]
        ContinueSlice(ContinueSlice),
        #[prost(message, tag = "16")]
        CheckpointExport(CheckpointExport),
        #[prost(message, tag = "17")]
        CheckpointImport(CheckpointImport),
        #[prost(message, tag = "18")]
        TimelineArchiveExport(TimelineArchiveExport),
        #[prost(message, tag = "19")]
        TimelineArchiveImport(TimelineArchiveImport),
        #[prost(message, tag = "20")]
        CapabilityNegotiation(CapabilityNegotiation),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct Response {
    #[prost(uint32, tag = "1")]
    pub protocol_version: u32,
    #[prost(uint64, tag = "2")]
    pub request_id: u64,
    #[prost(oneof = "response::Body", tags = "10, 11, 12, 13, 14, 15, 16, 17, 18")]
    pub body: Option<response::Body>,
}

pub mod response {
    use super::*;

    // Committed results are most responses; boxing them would allocate for each one.
    #[allow(clippy::large_enum_variant)]
    #[derive(Clone, PartialEq, Oneof)]
    pub enum Body {
        #[prost(message, tag = "10")]
        EngineCreated(EngineCreated),
        #[prost(message, tag = "11")]
        ProgramLoaded(ProgramLoaded),
        #[prost(message, tag = "12")]
        SessionCreated(SessionCreated),
        #[prost(message, tag = "13")]
        SliceYielded(SliceYielded),
        #[prost(message, tag = "14")]
        Committed(CommittedRunResult),
        #[prost(message, tag = "15")]
        Bundle(Bundle),
        #[prost(message, tag = "16")]
        Capabilities(CapabilityResult),
        #[prost(message, tag = "17")]
        Ack(Ack),
        #[prost(message, tag = "18")]
        Diagnostic(Diagnostic),
    }
}

#[derive(Clone, Copy, PartialEq, Message)]
pub struct EngineCreate {
    #[prost(uint64, tag = "1")]
    pub max_message_bytes: u64,
    #[prost(uint64, tag = "2")]
    pub max_slice_work: u64,
}

#[derive(Clone, Copy, PartialEq, Message)]
pub struct EngineCreated {
    #[prost(uint32, tag = "1")]
    pub abi_version: u32,
    #[prost(uint64, tag = "2")]
    pub max_message_bytes: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct ProgramLoad {
    #[prost(bytes = "vec", tag = "1")]
    pub artifact: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ProgramLoaded {
    #[prost(bytes = "vec", tag = "1")]
    pub artifact_id: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SessionCreate {
    #[prost(bytes = "vec", tag = "1")]
    pub artifact_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub execution_id: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SessionLoad {
    #[prost(bytes = "vec", tag = "1")]
    pub artifact_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub checkpoint_bundle: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Dispatch {
    #[prost(uint64, tag = "1")]
    pub session: u64,
    #[prost(message, optional, tag = "2")]
    pub input: Option<RuntimeInput>,
    #[prost(uint64, tag = "3")]
    pub slice_work: u64,
}

#[derive(Clone, Copy, PartialEq, Message)]
pub struct ContinueSlice {
    #[prost(uint64, tag = "1")]
    pub session: u64,
    #[prost(uint64, tag = "2")]
    pub slice_work: u64,
}

#[derive(Clone, Copy, PartialEq, Message)]
pub struct CheckpointExport {
    #[prost(uint64, tag = "1")]
    pub session: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct CheckpointImport {
    #[prost(bytes = "vec", tag = "1")]
    pub artifact_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub bundle: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Message)]
pub struct TimelineArchiveExport {
    #[prost(uint64, tag = "1")]
    pub session: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct TimelineArchiveImport {
    #[prost(bytes = "vec", tag = "1")]
    pub bundle: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct CapabilityNegotiation {
    #[prost(bytes = "vec", tag = "1")]
    pub artifact_id: Vec<u8>,
    #[prost(message, repeated, tag = "2")]
    pub host: Vec<Capability>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Capability {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(uint32, tag = "2")]
    pub version: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct CapabilityResult {
    #[prost(message, repeated, tag = "1")]
    pub accepted: Vec<Capability>,
}

#[derive(Clone, PartialEq, Message)]
pub struct RuntimeInput {
    #[prost(bytes = "vec", tag = "1")]
    pub request_id: Vec<u8>,
    #[prost(oneof = "runtime_input::Kind", tags = "10, 11, 12, 13, 14")]
    pub kind: Option<runtime_input::Kind>,
}

pub mod runtime_input {
    use super::*;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Kind {
        #[prost(message, tag = "10")]
        Start(Start),
        #[prost(message, tag = "11")]
        Advance(Advance),
        #[prost(message, tag = "12")]
        Select(Select),
        #[prost(message, tag = "13")]
        EffectResponse(EffectResponse),
        #[prost(message, tag = "14")]
        Event(Event),
    }
}

#[derive(Clone, Copy, PartialEq, Message)]
pub struct Start {}

#[derive(Clone, PartialEq, Message)]
pub struct Advance {
    #[prost(bytes = "vec", tag = "1")]
    pub interaction_id: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Select {
    #[prost(bytes = "vec", tag = "1")]
    pub interaction_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub choice_id: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct EffectResponse {
    #[prost(bytes = "vec", tag = "1")]
    pub effect_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub request_digest: Vec<u8>,
    #[prost(string, tag = "3")]
    pub capability: String,
    #[prost(uint32, tag = "4")]
    pub capability_version: u32,
    #[prost(bytes = "vec", tag = "5")]
    pub canonical_value: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Event {
    #[prost(bytes = "vec", tag = "1")]
    pub event_type_id: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SessionCreated {
    #[prost(uint64, tag = "1")]
    pub session: u64,
    #[prost(bytes = "vec", tag = "2")]
    pub commit_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub snapshot: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub execution_id: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Message)]
pub struct SliceYielded {
    #[prost(uint64, tag = "1")]
    pub session: u64,
    #[prost(uint64, tag = "2")]
    pub instruction_count: u64,
    #[prost(uint64, tag = "3")]
    pub call_count: u64,
    #[prost(uint64, tag = "4")]
    pub logical_alloc_units: u64,
    #[prost(uint64, tag = "5")]
    pub microstep_count: u64,
    #[prost(uint64, tag = "6")]
    pub internal_event_count: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct CommittedRunResult {
    #[prost(uint64, tag = "1")]
    pub session: u64,
    #[prost(bytes = "vec", tag = "2")]
    pub commit_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub receipt_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub snapshot: Vec<u8>,
    #[prost(bytes = "vec", tag = "5")]
    pub state_digest: Vec<u8>,
    #[prost(bool, tag = "6")]
    pub reused: bool,
    #[prost(message, optional, tag = "7")]
    pub result: Option<RunResult>,
}

/// Field 3 (`text`) is reserved: version 2 results carry content references (ADR 0018).
#[derive(Clone, PartialEq, Message)]
pub struct RunResult {
    #[prost(enumeration = "RunResultKind", tag = "1")]
    pub kind: i32,
    #[prost(bytes = "vec", tag = "2")]
    pub interaction_or_effect_id: Vec<u8>,
    #[prost(message, repeated, tag = "4")]
    pub choices: Vec<Choice>,
    #[prost(bytes = "vec", tag = "5")]
    pub canonical_value: Vec<u8>,
    #[prost(bytes = "vec", repeated, tag = "6")]
    pub active_states: Vec<Vec<u8>>,
    #[prost(message, optional, tag = "7")]
    pub speaker: Option<ContentRef>,
    #[prost(message, optional, tag = "8")]
    pub body: Option<Segment>,
    #[prost(message, optional, tag = "9")]
    pub prompt: Option<ContentRef>,
    #[prost(string, tag = "10")]
    pub capability: String,
    #[prost(uint64, tag = "11")]
    pub occurrence: u64,
}

#[derive(Clone, Copy, Debug, Eq, Enumeration, PartialEq)]
#[repr(i32)]
pub enum RunResultKind {
    Say = 0,
    Choice = 1,
    Effect = 2,
    Finished = 3,
    StatechartStable = 4,
}

/// Field 2 (the text `label`) is reserved.
#[derive(Clone, PartialEq, Message)]
pub struct Choice {
    #[prost(bytes = "vec", tag = "1")]
    pub id: Vec<u8>,
    #[prost(message, optional, tag = "3")]
    pub label: Option<ContentRef>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ContentRef {
    #[prost(string, tag = "1")]
    pub provider: String,
    #[prost(string, tag = "2")]
    pub key: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct Segment {
    #[prost(message, optional, tag = "1")]
    pub unit: Option<ContentRef>,
    #[prost(string, optional, tag = "2")]
    pub first: Option<String>,
    #[prost(string, optional, tag = "3")]
    pub last: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Bundle {
    #[prost(enumeration = "BundleKind", tag = "1")]
    pub kind: i32,
    #[prost(bytes = "vec", tag = "2")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, Enumeration, PartialEq)]
#[repr(i32)]
pub enum BundleKind {
    Checkpoint = 0,
    TimelineArchive = 1,
}

#[derive(Clone, Copy, PartialEq, Message)]
pub struct Ack {}

#[derive(Clone, PartialEq, Message)]
pub struct Diagnostic {
    #[prost(string, tag = "1")]
    pub code: String,
    #[prost(string, tag = "2")]
    pub class: String,
    #[prost(string, tag = "3")]
    pub message: String,
    #[prost(bool, tag = "4")]
    pub retryable: bool,
}

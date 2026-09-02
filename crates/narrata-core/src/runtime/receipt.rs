use crate::{
    codec::{CborWriter, ObjectKind, encode_envelope},
    identity::{InputId, InputPayloadDigest, StateDigest},
    version::{RECEIPT_SCHEMA_V0, ReceiptSchemaVersion},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ReceiptResultKindV0 {
    Say = 0,
    Choice = 1,
    Finished = 2,
    Effect = 3,
    StatechartStable = 4,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionReceiptV0 {
    pub schema_version: ReceiptSchemaVersion,
    pub request_id: InputId,
    pub input_payload_digest: InputPayloadDigest,
    pub parent_state: StateDigest,
    pub next_state: StateDigest,
    pub result_kind: ReceiptResultKindV0,
    pub instruction_count: u64,
    pub call_count: u64,
    pub logical_alloc_units: u64,
    pub microstep_count: u64,
    pub internal_event_count: u64,
}

pub fn encode_receipt(receipt: &TransitionReceiptV0) -> Vec<u8> {
    let payload = encode_receipt_payload(receipt);
    encode_envelope(ObjectKind::Receipt, receipt.schema_version.get(), &payload)
}

pub(crate) fn encode_receipt_payload(receipt: &TransitionReceiptV0) -> Vec<u8> {
    let mut writer = CborWriter::new();
    let has_statechart_metrics = receipt.microstep_count != 0 || receipt.internal_event_count != 0;
    writer.map(if has_statechart_metrics { 11 } else { 9 });
    writer.unsigned(0);
    writer.unsigned(u64::from(receipt.schema_version.get()));
    writer.unsigned(1);
    writer.bytes(receipt.request_id.as_bytes());
    writer.unsigned(2);
    writer.bytes(receipt.input_payload_digest.as_bytes());
    writer.unsigned(3);
    writer.bytes(receipt.parent_state.as_bytes());
    writer.unsigned(4);
    writer.bytes(receipt.next_state.as_bytes());
    writer.unsigned(5);
    writer.unsigned(receipt.result_kind as u64);
    writer.unsigned(6);
    writer.unsigned(receipt.instruction_count);
    writer.unsigned(7);
    writer.unsigned(receipt.call_count);
    writer.unsigned(8);
    writer.unsigned(receipt.logical_alloc_units);
    if has_statechart_metrics {
        writer.unsigned(9);
        writer.unsigned(receipt.microstep_count);
        writer.unsigned(10);
        writer.unsigned(receipt.internal_event_count);
    }
    writer.into_bytes()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReceiptMetrics {
    pub instruction_count: u64,
    pub call_count: u64,
    pub logical_alloc_units: u64,
    pub microstep_count: u64,
    pub internal_event_count: u64,
}

pub(crate) fn receipt_v0(
    request_id: InputId,
    input_payload_digest: InputPayloadDigest,
    parent_state: StateDigest,
    next_state: StateDigest,
    result_kind: ReceiptResultKindV0,
    metrics: ReceiptMetrics,
) -> TransitionReceiptV0 {
    TransitionReceiptV0 {
        schema_version: RECEIPT_SCHEMA_V0,
        request_id,
        input_payload_digest,
        parent_state,
        next_state,
        result_kind,
        instruction_count: metrics.instruction_count,
        call_count: metrics.call_count,
        logical_alloc_units: metrics.logical_alloc_units,
        microstep_count: metrics.microstep_count,
        internal_event_count: metrics.internal_event_count,
    }
}

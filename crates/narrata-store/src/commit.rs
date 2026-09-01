use narrata_core::{
    CommitId, ExecutionId, InputId, InputPayloadDigest, ProgramArtifactId, ReceiptId, SnapshotId,
    StateDigest,
    codec::ObjectKind,
    runtime::{ReceiptResultKindV0, TransitionDraft, Turn},
};

use crate::{
    CheckedObject,
    codec::{Reader, WireError, Writer, expect_map, key},
};

pub const COMMIT_SCHEMA_V1: u16 = 1;
pub const STORED_RECEIPT_SCHEMA_V1: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitCauseV1 {
    Genesis,
    RuntimeTransition(ReceiptId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitV1 {
    pub parent: Option<CommitId>,
    pub execution: ExecutionId,
    pub program: ProgramArtifactId,
    pub snapshot: SnapshotId,
    pub cause: CommitCauseV1,
    pub ledger_fence: u64,
    pub turn: Turn,
}

impl CommitV1 {
    pub fn to_object(&self) -> CheckedObject {
        CheckedObject::new(ObjectKind::Commit, COMMIT_SCHEMA_V1, &self.encode())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.map(8);
        writer.unsigned(0);
        writer.unsigned(u64::from(COMMIT_SCHEMA_V1));
        writer.unsigned(1);
        match self.parent {
            Some(parent) => writer.bytes(parent.as_bytes()),
            None => writer.null(),
        }
        writer.unsigned(2);
        writer.bytes(self.execution.as_bytes());
        writer.unsigned(3);
        writer.bytes(self.program.as_bytes());
        writer.unsigned(4);
        writer.bytes(self.snapshot.as_bytes());
        writer.unsigned(5);
        match self.cause {
            CommitCauseV1::Genesis => {
                writer.array(1);
                writer.unsigned(0);
            }
            CommitCauseV1::RuntimeTransition(receipt) => {
                writer.array(2);
                writer.unsigned(1);
                writer.bytes(receipt.as_bytes());
            }
        }
        writer.unsigned(6);
        writer.unsigned(self.ledger_fence);
        writer.unsigned(7);
        writer.unsigned(self.turn.0);
        writer.into_bytes()
    }

    pub fn decode(payload: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(payload);
        expect_map(&mut reader, 8)?;
        key(&mut reader, 0)?;
        if reader.unsigned()? != u64::from(COMMIT_SCHEMA_V1) {
            return Err(WireError::Schema("Commit schema"));
        }
        key(&mut reader, 1)?;
        let parent =
            reader.optional(|reader| Ok(CommitId::from_bytes(reader.bytes_exact::<32>()?)))?;
        key(&mut reader, 2)?;
        let execution = ExecutionId::from_bytes(reader.bytes_exact::<16>()?);
        key(&mut reader, 3)?;
        let program = ProgramArtifactId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 4)?;
        let snapshot = SnapshotId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 5)?;
        let cause_len = reader.array()?;
        let cause = match (reader.unsigned()?, cause_len) {
            (0, 1) => CommitCauseV1::Genesis,
            (1, 2) => {
                CommitCauseV1::RuntimeTransition(ReceiptId::from_bytes(reader.bytes_exact::<32>()?))
            }
            _ => return Err(WireError::Schema("Commit cause")),
        };
        key(&mut reader, 6)?;
        let ledger_fence = reader.unsigned()?;
        key(&mut reader, 7)?;
        let turn = Turn(reader.unsigned()?);
        reader.finish()?;
        let value = Self {
            parent,
            execution,
            program,
            snapshot,
            cause,
            ledger_fence,
            turn,
        };
        if value.encode() != payload {
            return Err(WireError::NonCanonical);
        }
        Ok(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionReceiptV1 {
    pub execution: ExecutionId,
    pub program: ProgramArtifactId,
    pub parent: CommitId,
    pub request_id: InputId,
    pub input_payload_digest: InputPayloadDigest,
    pub parent_state: StateDigest,
    pub next_state: StateDigest,
    pub next_snapshot: SnapshotId,
    pub result_kind: ReceiptResultKindV0,
    pub instruction_count: u64,
    pub call_count: u64,
    pub logical_alloc_units: u64,
}

impl TransitionReceiptV1 {
    pub fn from_draft(
        execution: ExecutionId,
        program: ProgramArtifactId,
        parent: CommitId,
        next_snapshot: SnapshotId,
        draft: &TransitionDraft,
    ) -> Self {
        let receipt = draft.receipt();
        Self {
            execution,
            program,
            parent,
            request_id: receipt.request_id,
            input_payload_digest: receipt.input_payload_digest,
            parent_state: receipt.parent_state,
            next_state: receipt.next_state,
            next_snapshot,
            result_kind: receipt.result_kind,
            instruction_count: receipt.instruction_count,
            call_count: receipt.call_count,
            logical_alloc_units: receipt.logical_alloc_units,
        }
    }

    pub fn to_object(&self) -> CheckedObject {
        CheckedObject::new(
            ObjectKind::Receipt,
            STORED_RECEIPT_SCHEMA_V1,
            &self.encode(),
        )
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.map(13);
        writer.unsigned(0);
        writer.unsigned(u64::from(STORED_RECEIPT_SCHEMA_V1));
        writer.unsigned(1);
        writer.bytes(self.execution.as_bytes());
        writer.unsigned(2);
        writer.bytes(self.program.as_bytes());
        writer.unsigned(3);
        writer.bytes(self.parent.as_bytes());
        writer.unsigned(4);
        writer.bytes(self.request_id.as_bytes());
        writer.unsigned(5);
        writer.bytes(self.input_payload_digest.as_bytes());
        writer.unsigned(6);
        writer.bytes(self.parent_state.as_bytes());
        writer.unsigned(7);
        writer.bytes(self.next_state.as_bytes());
        writer.unsigned(8);
        writer.bytes(self.next_snapshot.as_bytes());
        writer.unsigned(9);
        writer.unsigned(self.result_kind as u64);
        writer.unsigned(10);
        writer.unsigned(self.instruction_count);
        writer.unsigned(11);
        writer.unsigned(self.call_count);
        writer.unsigned(12);
        writer.unsigned(self.logical_alloc_units);
        writer.into_bytes()
    }

    pub fn decode(payload: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(payload);
        expect_map(&mut reader, 13)?;
        key(&mut reader, 0)?;
        if reader.unsigned()? != u64::from(STORED_RECEIPT_SCHEMA_V1) {
            return Err(WireError::Schema("Receipt schema"));
        }
        key(&mut reader, 1)?;
        let execution = ExecutionId::from_bytes(reader.bytes_exact::<16>()?);
        key(&mut reader, 2)?;
        let program = ProgramArtifactId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 3)?;
        let parent = CommitId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 4)?;
        let request_id = InputId::from_bytes(reader.bytes_exact::<16>()?);
        key(&mut reader, 5)?;
        let input_payload_digest = InputPayloadDigest::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 6)?;
        let parent_state = StateDigest::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 7)?;
        let next_state = StateDigest::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 8)?;
        let next_snapshot = SnapshotId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 9)?;
        let result_kind = match reader.unsigned()? {
            0 => ReceiptResultKindV0::Say,
            1 => ReceiptResultKindV0::Choice,
            2 => ReceiptResultKindV0::Finished,
            _ => return Err(WireError::Schema("Receipt result kind")),
        };
        key(&mut reader, 10)?;
        let instruction_count = reader.unsigned()?;
        key(&mut reader, 11)?;
        let call_count = reader.unsigned()?;
        key(&mut reader, 12)?;
        let logical_alloc_units = reader.unsigned()?;
        reader.finish()?;
        let value = Self {
            execution,
            program,
            parent,
            request_id,
            input_payload_digest,
            parent_state,
            next_state,
            next_snapshot,
            result_kind,
            instruction_count,
            call_count,
            logical_alloc_units,
        };
        if value.encode() != payload {
            return Err(WireError::NonCanonical);
        }
        Ok(value)
    }
}

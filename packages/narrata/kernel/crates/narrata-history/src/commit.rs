//! The commit format every registered domain shares, and the registration of a domain's kinds
//! (ADR 0015).

use std::{convert::Infallible, marker::PhantomData};

use narrata_kernel::codec::{CborReader, CborWriter, DecodeError, DecodeLimits, decode_checked};
use narrata_storage::{Conflict, Expect, StorageBackend};

use crate::{
    ArtifactId, Domain, HistoryError, KindInfo, Object, ObjectId, Op, Reader, Reference, Registry,
    Tag, View,
    layout::{self, expect_map, key},
};

pub const COMMIT_SCHEMA: u16 = 1;

/// `{0: artifact, 1: parent | null, 2: input | null, 3: state, 4: depth}`, stored under the
/// domain's commit kind; the identity is `object_id(kind, 1, payload)`. A root has neither parent
/// nor input and depth 0; every other commit has both and its parent's depth plus one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Commit {
    pub artifact: ArtifactId,
    pub parent: Option<ObjectId>,
    pub input: Option<ObjectId>,
    pub state: ObjectId,
    pub depth: u64,
}

impl Commit {
    pub const fn root(artifact: ArtifactId, state: ObjectId) -> Self {
        Self {
            artifact,
            parent: None,
            input: None,
            state,
            depth: 0,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut writer = CborWriter::new();
        writer.map(5);
        writer.unsigned(0);
        writer.bytes(self.artifact.as_bytes());
        writer.unsigned(1);
        optional_id(&mut writer, self.parent);
        writer.unsigned(2);
        optional_id(&mut writer, self.input);
        writer.unsigned(3);
        writer.bytes(self.state.as_bytes());
        writer.unsigned(4);
        writer.unsigned(self.depth);
        writer.into_bytes()
    }

    pub fn decode(payload: &[u8]) -> Result<Self, DecodeError> {
        decode_checked(
            payload,
            &DecodeLimits::default(),
            |reader| {
                expect_map(reader, 5)?;
                key(reader, 0)?;
                let artifact = ArtifactId::from_bytes(reader.bytes_exact::<32>()?);
                key(reader, 1)?;
                let parent = read_optional_id(reader)?;
                key(reader, 2)?;
                let input = read_optional_id(reader)?;
                key(reader, 3)?;
                let state = ObjectId::from_bytes(reader.bytes_exact::<32>()?);
                key(reader, 4)?;
                let depth = reader.unsigned()?;
                let root = parent.is_none();
                if input.is_none() != root || (depth == 0) != root {
                    return Err(DecodeError::Schema("commit root shape"));
                }
                Ok(Self {
                    artifact,
                    parent,
                    input,
                    state,
                    depth,
                })
            },
            Self::encode,
        )
    }

    pub fn to_object(self, kind: u16) -> Object {
        Object::new(kind, COMMIT_SCHEMA, &self.encode())
    }

    /// Decodes a stored commit of `kind`; a wrong kind or schema is
    /// [`HistoryError::ObjectKind`], a malformed payload [`HistoryError::Corrupt`].
    pub fn from_object(object: &Object, kind: u16) -> Result<Self, HistoryError> {
        if object.kind() != kind || object.schema() != COMMIT_SCHEMA {
            return Err(HistoryError::ObjectKind(object.id()));
        }
        Self::decode(object.payload())
            .map_err(|error| HistoryError::Corrupt(object.id(), error.to_string()))
    }
}

fn optional_id(writer: &mut CborWriter, id: Option<ObjectId>) {
    match id {
        Some(id) => writer.bytes(id.as_bytes()),
        None => writer.null(),
    }
}

fn read_optional_id(reader: &mut CborReader<'_>) -> Result<Option<ObjectId>, DecodeError> {
    reader.optional(|reader| Ok(ObjectId::from_bytes(reader.bytes_exact::<32>()?)))
}

/// What the engine knows about the kinds of domain `D`.
pub fn kind<D: Domain>(code: u16) -> Option<KindInfo> {
    match code {
        _ if code == D::COMMIT_KIND => Some(KindInfo {
            name: "commit",
            leaf: false,
            commit: true,
        }),
        _ if code == D::STATE_KIND => Some(KindInfo {
            name: "state",
            leaf: true,
            commit: false,
        }),
        _ if code == D::INPUT_KIND => Some(KindInfo {
            name: "input",
            leaf: true,
            commit: false,
        }),
        _ => None,
    }
}

/// A commit refers to its parent, its input and its state; states and inputs refer to nothing.
pub fn references<D: Domain>(object: &Object) -> Result<Vec<Reference>, HistoryError> {
    if object.kind() != D::COMMIT_KIND {
        return Ok(Vec::new());
    }
    let commit = Commit::from_object(object, D::COMMIT_KIND)?;
    let mut values = vec![Reference::object(commit.state, D::STATE_KIND)];
    values.extend(
        commit
            .parent
            .map(|id| Reference::object(id, D::COMMIT_KIND)),
    );
    values.extend(commit.input.map(|id| Reference::object(id, D::INPUT_KIND)));
    Ok(values)
}

/// Checks a new object of domain `D`. A commit must extend a parent of the same kind and
/// artifact by one step, and may not contradict the transition already recorded for its parent
/// and input; it adds its children and transitions index keys. States and inputs are checked for
/// their schema only: decoding them needs the artifact, which the session that loads them has.
pub fn validate<D: Domain, B: StorageBackend, R: Registry>(
    object: &Object,
    view: &mut View<'_, B, R>,
) -> Result<Vec<Op<R::Tag>>, HistoryError> {
    let id = object.id();
    let schema = |expected: u16| {
        if object.schema() == expected {
            Ok(Vec::new())
        } else {
            Err(HistoryError::ObjectKind(id))
        }
    };
    match object.kind() {
        kind if kind == D::STATE_KIND => return schema(D::STATE_SCHEMA),
        kind if kind == D::INPUT_KIND => return schema(D::INPUT_SCHEMA),
        kind if kind == D::COMMIT_KIND => {}
        _ => return Err(HistoryError::ObjectKind(id)),
    }
    let commit = Commit::from_object(object, D::COMMIT_KIND)?;
    let (Some(parent_id), Some(input)) = (commit.parent, commit.input) else {
        return Ok(Vec::new());
    };
    let parent = view.object(parent_id)?;
    if let Some(parent) = parent {
        let parent = Commit::from_object(&parent, D::COMMIT_KIND)?;
        if parent.artifact != commit.artifact {
            return Err(HistoryError::InvalidGraph(
                "commit names another artifact than its parent",
            ));
        }
        if parent.depth.checked_add(1) != Some(commit.depth) {
            return Err(HistoryError::InvalidGraph(
                "commit depth is not its parent's plus one",
            ));
        }
    } else {
        view.require_truncated_parent(parent_id)?;
    }
    let mut index = vec![Op::put(
        layout::CHILDREN,
        layout::child_key(parent_id, id),
        layout::encode_marker(),
        Expect::Any,
        Tag::Index,
    )];
    let key = layout::transition_key(parent_id, input);
    let recorded = match view.planned(layout::TRANSITIONS, &key) {
        Some(value) => Some(layout::decode_transition(value)?),
        None => view
            .reader()
            .read_key(layout::TRANSITIONS, &key)?
            .map(|value| layout::decode_transition(&value.value))
            .transpose()?,
    };
    match recorded {
        Some(recorded) if recorded == id => {}
        Some(recorded) => {
            return Err(crate::Nondeterminism {
                parent: parent_id,
                input,
                recorded,
                proposed: id,
            }
            .into());
        }
        None => {
            let value = layout::encode_transition(id);
            view.plan(layout::TRANSITIONS, key.clone(), value.clone());
            index.push(Op::put(
                layout::TRANSITIONS,
                key,
                value,
                Expect::Absent,
                Tag::Replan,
            ));
        }
    }
    Ok(index)
}

/// The index keys a commit of domain `D` leaves behind when GC deletes it.
pub fn unindex<D: Domain, B: StorageBackend, T>(
    object: &Object,
    reader: &Reader<'_, B>,
) -> Result<Vec<Op<T>>, HistoryError> {
    if object.kind() != D::COMMIT_KIND {
        return Ok(Vec::new());
    }
    let Ok(commit) = Commit::from_object(object, D::COMMIT_KIND) else {
        return Ok(Vec::new());
    };
    let (Some(parent), Some(input)) = (commit.parent, commit.input) else {
        return Ok(Vec::new());
    };
    let mut ops = vec![Op::delete(
        layout::CHILDREN,
        layout::child_key(parent, object.id()),
        Expect::Any,
        Tag::Index,
    )];
    let key = layout::transition_key(parent, input);
    if let Some(value) = reader.read_key(layout::TRANSITIONS, &key)?
        && layout::decode_transition(&value.value)? == object.id()
    {
        ops.push(Op::delete(
            layout::TRANSITIONS,
            key,
            Expect::Revision(value.revision),
            Tag::Replan,
        ));
    }
    Ok(ops)
}

/// The registry of a store that holds one domain's sessions.
pub struct DomainKinds<D>(PhantomData<fn() -> D>);

impl<D> DomainKinds<D> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<D> Default for DomainKinds<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D> std::fmt::Debug for DomainKinds<D> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DomainKinds")
    }
}

impl<D: Domain> Registry for DomainKinds<D> {
    type Error = HistoryError;
    type Tag = Infallible;

    fn kind(&self, code: u16) -> Option<KindInfo> {
        kind::<D>(code)
    }

    fn references(&self, object: &Object) -> Result<Vec<Reference>, HistoryError> {
        references::<D>(object)
    }

    fn validate<B: StorageBackend>(
        &self,
        object: &Object,
        view: &mut View<'_, B, Self>,
    ) -> Result<Vec<Op<Infallible>>, HistoryError> {
        validate::<D, B, Self>(object, view)
    }

    fn unindex<B: StorageBackend>(
        &self,
        object: &Object,
        reader: &Reader<'_, B>,
    ) -> Result<Vec<Op<Infallible>>, HistoryError> {
        unindex::<D, B, Infallible>(object, reader)
    }

    fn conflict(&self, tag: &Infallible, _: Conflict) -> HistoryError {
        match *tag {}
    }
}

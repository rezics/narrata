//! ADR 0003 canonical CBOR for node objects. Field numbers are fixed here and by the
//! `fixtures/compat/nodes-r2` and `fixtures/compat/nodes-r2-proposals` corpora. Maps keyed by names or IDs are arrays of `[key, value]`
//! pairs in strictly increasing key order, because profile maps only take integer keys.
//! Decoders check structure and bounded sizes; `check` verifies references and types.

use std::collections::{BTreeMap, BTreeSet};

use narrata_kernel::{
    codec::{
        CborReader, CborWriter, DecodeError, DecodeLimits, EnvelopeLimits, decode_checked,
        decode_envelope, encode_envelope, object_id,
    },
    content::{AnchorId, ContentRef, Segment},
};

use crate::{
    ArtifactId, Assignment, BinaryOp, ChoicePointId, CommitId, Error, Expr, MAX_ARGS,
    MAX_ASSIGNMENTS, MAX_CALL_DEPTH, MAX_CHOICE_POINTS, MAX_GRAPH_NODES, MAX_NAME_BYTES,
    MAX_OPTIONS, MAX_SHARED, MAX_TEXT_BYTES, MAX_VARIABLES, NodeId, ObjectId, OptionId, Result,
    Scalar, ScalarType, Scope, Variable,
    expr::MAX_EXPRESSION_DEPTH,
    plan::{
        CallTarget, ChoicePoint, Chunk, Ending, Graph, GraphEntry, GraphHeader, GraphNames,
        GraphRef, ImportRef, Manifest, NameTable, OptionPlan, Outcome, PackageInstance, Passage,
        Plan, ProductHeader, SharedVariable, Signature, TombstoneSet,
    },
    proposal::{MAX_OVERLAY_NODES, MAX_PROPOSED_NODES, MAX_PROPOSED_OPTIONS},
    source::R1ArtifactId,
    state::{Commit, Finished, Frame, Input, Overlay, State},
    valid_name,
};

pub const KIND_MANIFEST: u16 = 0x0100;
pub const KIND_CHUNK: u16 = 0x0101;
pub const KIND_TOMBSTONES: u16 = 0x0102;
pub const KIND_NAMES: u16 = 0x0103;
pub const KIND_PACK: u16 = 0x0104;
pub const KIND_COMMIT: u16 = 0x0110;
pub const KIND_STATE: u16 = 0x0111;
pub const KIND_INPUT: u16 = 0x0112;
/// Every node object kind starts at schema 1.
pub const SCHEMA: u16 = 1;

type D<T> = std::result::Result<T, DecodeError>;

const MAX_PACKAGE_ID_BYTES: u64 = 128;
const MAX_VERSION_BYTES: u64 = 64;
const MAX_TYPE_ID_BYTES: u64 = 128;
const MAX_REVISION_BYTES: u64 = 64;

/// The `object_id` and envelope of a payload.
pub(crate) fn seal(kind: u16, payload: &[u8]) -> (ObjectId, Vec<u8>) {
    (
        ObjectId::from_bytes(object_id(kind, SCHEMA, payload)),
        encode_envelope(kind, SCHEMA, payload),
    )
}

pub(crate) fn id_of(kind: u16, payload: &[u8]) -> ObjectId {
    ObjectId::from_bytes(object_id(kind, SCHEMA, payload))
}

/// Opens an envelope of `kind` and decodes its payload with the checked decoder.
pub(crate) fn open<T>(
    bytes: &[u8],
    kind: u16,
    max_payload: usize,
    what: &str,
    decode: impl for<'a> FnOnce(&mut CborReader<'a>) -> D<T>,
    encode: impl FnOnce(&T) -> Vec<u8>,
) -> Result<(T, ObjectId)> {
    let fail = |error: DecodeError| Error::new("decode", what, error.to_string());
    let envelope = decode_envelope(
        bytes,
        kind,
        SCHEMA,
        &EnvelopeLimits {
            max_envelope_bytes: max_payload as u64 + 56,
            max_payload_bytes: max_payload as u64,
        },
    )
    .map_err(fail)?;
    let limits = DecodeLimits {
        max_payload_bytes: max_payload as u64,
        max_string_bytes: max_payload as u64,
        ..DecodeLimits::default()
    };
    let value = decode_checked(envelope.payload, &limits, decode, encode).map_err(fail)?;
    Ok((value, id_of(kind, envelope.payload)))
}

pub(crate) struct MapIn<'r, 'a> {
    reader: &'r mut CborReader<'a>,
    remaining: u64,
    previous: Option<u64>,
}

impl<'r, 'a> MapIn<'r, 'a> {
    pub fn new(reader: &'r mut CborReader<'a>, max: u64) -> D<Self> {
        let remaining = reader.map_len()?;
        if remaining > max {
            return Err(DecodeError::Schema("unexpected map fields"));
        }
        Ok(Self {
            reader,
            remaining,
            previous: None,
        })
    }

    pub fn next(&mut self) -> D<Option<u64>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        let key = self.reader.unsigned()?;
        if self.previous.is_some_and(|previous| key <= previous) {
            return Err(DecodeError::NonCanonical("map key order"));
        }
        self.previous = Some(key);
        Ok(Some(key))
    }

    pub fn r(&mut self) -> &mut CborReader<'a> {
        self.reader
    }
}

fn unknown() -> DecodeError {
    DecodeError::Schema("unknown field")
}

fn required<T>(value: Option<T>, field: &'static str) -> D<T> {
    value.ok_or(DecodeError::Schema(field))
}

fn map_header(w: &mut CborWriter, present: &[bool]) {
    w.map(present.iter().filter(|value| **value).count() as u64);
}

fn exact_array(r: &mut CborReader<'_>, length: u64) -> D<()> {
    if r.array_len()? == length {
        Ok(())
    } else {
        Err(DecodeError::Schema("array length"))
    }
}

fn array<'a, T>(
    r: &mut CborReader<'a>,
    max: usize,
    mut item: impl FnMut(&mut CborReader<'a>) -> D<T>,
) -> D<Vec<T>> {
    let length = r.array_len()?;
    if length > max as u64 {
        return Err(DecodeError::Limit("collection length"));
    }
    let mut values = Vec::with_capacity(length as usize);
    for _ in 0..length {
        values.push(item(r)?);
    }
    Ok(values)
}

fn sorted_set<'a, T: Ord>(
    r: &mut CborReader<'a>,
    max: usize,
    item: impl FnMut(&mut CborReader<'a>) -> D<T>,
) -> D<BTreeSet<T>> {
    let values = array(r, max, item)?;
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(DecodeError::NonCanonical("set order"));
    }
    Ok(values.into_iter().collect())
}

fn pairs<'a, K: Ord, V>(
    r: &mut CborReader<'a>,
    max: usize,
    mut key: impl FnMut(&mut CborReader<'a>) -> D<K>,
    mut value: impl FnMut(&mut CborReader<'a>) -> D<V>,
) -> D<BTreeMap<K, V>> {
    let length = r.array_len()?;
    if length > max as u64 {
        return Err(DecodeError::Limit("collection length"));
    }
    let mut values = BTreeMap::new();
    for _ in 0..length {
        exact_array(r, 2)?;
        let k = key(r)?;
        if values.last_key_value().is_some_and(|(last, _)| &k <= last) {
            return Err(DecodeError::NonCanonical("entry order"));
        }
        let v = value(r)?;
        values.insert(k, v);
    }
    Ok(values)
}

fn write_pairs<K, V>(
    w: &mut CborWriter,
    map: &BTreeMap<K, V>,
    mut key: impl FnMut(&mut CborWriter, &K),
    mut value: impl FnMut(&mut CborWriter, &V),
) {
    w.array(map.len() as u64);
    for (k, v) in map {
        w.array(2);
        key(w, k);
        value(w, v);
    }
}

fn write_name(w: &mut CborWriter, value: &impl AsRef<str>) {
    w.text(value.as_ref());
}

fn name(r: &mut CborReader<'_>) -> D<String> {
    let text = r.text(MAX_NAME_BYTES as u64)?;
    if valid_name(text) {
        Ok(text.to_owned())
    } else {
        Err(DecodeError::Schema("identifier"))
    }
}

fn bounded_text(r: &mut CborReader<'_>, max: u64) -> D<String> {
    let text = r.text(max)?;
    if text.is_empty() {
        return Err(DecodeError::Schema("empty text"));
    }
    Ok(text.to_owned())
}

fn u32_value(r: &mut CborReader<'_>) -> D<u32> {
    u32::try_from(r.unsigned()?).map_err(|_| DecodeError::Schema("u32 range"))
}

fn u16_value(r: &mut CborReader<'_>) -> D<u16> {
    u16::try_from(r.unsigned()?).map_err(|_| DecodeError::Schema("u16 range"))
}

fn node_id(r: &mut CborReader<'_>) -> D<NodeId> {
    r.bytes_exact::<16>().map(NodeId::from_bytes)
}

fn choice_point_id(r: &mut CborReader<'_>) -> D<ChoicePointId> {
    r.bytes_exact::<16>().map(ChoicePointId::from_bytes)
}

fn option_id(r: &mut CborReader<'_>) -> D<OptionId> {
    r.bytes_exact::<16>().map(OptionId::from_bytes)
}

fn object(r: &mut CborReader<'_>) -> D<ObjectId> {
    r.bytes_exact::<32>().map(ObjectId::from_bytes)
}

fn anchor(r: &mut CborReader<'_>) -> D<AnchorId> {
    let text = r.text(narrata_kernel::content::MAX_ANCHOR_BYTES as u64)?;
    AnchorId::new(text).map_err(|_| DecodeError::Schema("content anchor"))
}

// --- graph references and signatures ---

fn write_graph_ref(w: &mut CborWriter, value: &GraphRef) {
    w.array(2);
    w.text(&value.package);
    w.text(&value.graph);
}

fn graph_ref(r: &mut CborReader<'_>) -> D<GraphRef> {
    exact_array(r, 2)?;
    Ok(GraphRef {
        package: name(r)?,
        graph: name(r)?,
    })
}

fn write_import_ref(w: &mut CborWriter, value: &ImportRef) {
    w.array(3);
    w.text(&value.package);
    w.text(&value.graph);
    w.text(&value.port);
}

fn import_ref(r: &mut CborReader<'_>) -> D<ImportRef> {
    exact_array(r, 3)?;
    Ok(ImportRef {
        package: name(r)?,
        graph: name(r)?,
        port: name(r)?,
    })
}

fn write_scalar_type(w: &mut CborWriter, value: &ScalarType) {
    w.unsigned(match value {
        ScalarType::Bool => 0,
        ScalarType::Int => 1,
        ScalarType::Text => 2,
        ScalarType::Ref => 3,
    });
}

fn scalar_type(r: &mut CborReader<'_>) -> D<ScalarType> {
    match r.unsigned()? {
        0 => Ok(ScalarType::Bool),
        1 => Ok(ScalarType::Int),
        2 => Ok(ScalarType::Text),
        3 => Ok(ScalarType::Ref),
        _ => Err(DecodeError::Schema("scalar type")),
    }
}

fn write_outcomes(w: &mut CborWriter, outcomes: &[String]) {
    w.array(outcomes.len() as u64);
    for outcome in outcomes {
        w.text(outcome);
    }
}

fn outcomes(r: &mut CborReader<'_>) -> D<Vec<String>> {
    Ok(sorted_set(r, MAX_VARIABLES, name)?.into_iter().collect())
}

fn write_signature(w: &mut CborWriter, value: &Signature) {
    w.map(2);
    w.unsigned(0);
    write_pairs(w, &value.parameters, write_name, write_scalar_type);
    w.unsigned(1);
    write_outcomes(w, &value.outcomes);
}

fn signature(r: &mut CborReader<'_>) -> D<Signature> {
    let mut parameters = None;
    let mut outcome_names = None;
    let mut map = MapIn::new(r, 2)?;
    while let Some(key) = map.next()? {
        match key {
            0 => parameters = Some(pairs(map.r(), MAX_VARIABLES, name, scalar_type)?),
            1 => outcome_names = Some(outcomes(map.r())?),
            _ => return Err(unknown()),
        }
    }
    Ok(Signature {
        parameters: required(parameters, "signature parameters")?,
        outcomes: required(outcome_names, "signature outcomes")?,
    })
}

// --- values and expressions ---

pub(crate) fn write_scalar(w: &mut CborWriter, value: &Scalar) {
    w.array(2);
    match value {
        Scalar::Bool(value) => {
            w.unsigned(0);
            w.boolean(*value);
        }
        Scalar::Int(value) => {
            w.unsigned(1);
            w.signed(*value);
        }
        Scalar::Text(value) => {
            w.unsigned(2);
            w.text(value);
        }
        Scalar::Ref(value) => {
            w.unsigned(3);
            value.encode(w);
        }
    }
}

pub(crate) fn scalar(r: &mut CborReader<'_>) -> D<Scalar> {
    exact_array(r, 2)?;
    match r.unsigned()? {
        0 => Ok(Scalar::Bool(r.boolean()?)),
        1 => Ok(Scalar::Int(r.signed()?)),
        2 => Ok(Scalar::Text(r.text(MAX_TEXT_BYTES as u64)?.to_owned())),
        3 => Ok(Scalar::Ref(ContentRef::decode(r)?)),
        _ => Err(DecodeError::Schema("scalar tag")),
    }
}

fn write_scope(w: &mut CborWriter, value: Scope) {
    w.unsigned(match value {
        Scope::Parameter => 0,
        Scope::Local => 1,
        Scope::Shared => 2,
    });
}

fn scope(r: &mut CborReader<'_>) -> D<Scope> {
    match r.unsigned()? {
        0 => Ok(Scope::Parameter),
        1 => Ok(Scope::Local),
        2 => Ok(Scope::Shared),
        _ => Err(DecodeError::Schema("scope")),
    }
}

const OPERATORS: [BinaryOp; 10] = [
    BinaryOp::Eq,
    BinaryOp::Ne,
    BinaryOp::Lt,
    BinaryOp::Le,
    BinaryOp::Gt,
    BinaryOp::Ge,
    BinaryOp::Add,
    BinaryOp::Sub,
    BinaryOp::And,
    BinaryOp::Or,
];

fn write_expr(w: &mut CborWriter, value: &Expr) {
    match value {
        Expr::Literal { value } => {
            w.array(2);
            w.unsigned(0);
            write_scalar(w, value);
        }
        Expr::Read { scope, name } => {
            w.array(3);
            w.unsigned(1);
            write_scope(w, *scope);
            w.text(name);
        }
        Expr::Not { value } => {
            w.array(2);
            w.unsigned(2);
            write_expr(w, value);
        }
        Expr::Binary { op, left, right } => {
            w.array(4);
            w.unsigned(3);
            let index = OPERATORS.iter().position(|value| value == op).unwrap_or(0);
            w.unsigned(index as u64);
            write_expr(w, left);
            write_expr(w, right);
        }
    }
}

fn expr(r: &mut CborReader<'_>) -> D<Expr> {
    expr_at(r, 0)
}

fn expr_at(r: &mut CborReader<'_>, depth: usize) -> D<Expr> {
    if depth > MAX_EXPRESSION_DEPTH {
        return Err(DecodeError::Limit("expression depth"));
    }
    let length = r.array_len()?;
    match (r.unsigned()?, length) {
        (0, 2) => Ok(Expr::Literal { value: scalar(r)? }),
        (1, 3) => Ok(Expr::Read {
            scope: scope(r)?,
            name: name(r)?,
        }),
        (2, 2) => Ok(Expr::Not {
            value: Box::new(expr_at(r, depth + 1)?),
        }),
        (3, 4) => {
            let op = usize::try_from(r.unsigned()?)
                .ok()
                .and_then(|index| OPERATORS.get(index).copied())
                .ok_or(DecodeError::Schema("binary operator"))?;
            Ok(Expr::Binary {
                op,
                left: Box::new(expr_at(r, depth + 1)?),
                right: Box::new(expr_at(r, depth + 1)?),
            })
        }
        _ => Err(DecodeError::Schema("expression")),
    }
}

fn write_assignments(w: &mut CborWriter, values: &[Assignment]) {
    w.array(values.len() as u64);
    for value in values {
        w.array(3);
        write_scope(w, value.target.scope);
        w.text(&value.target.name);
        write_expr(w, &value.value);
    }
}

fn assignments(r: &mut CborReader<'_>) -> D<Vec<Assignment>> {
    array(r, MAX_ASSIGNMENTS, |r| {
        exact_array(r, 3)?;
        Ok(Assignment {
            target: Variable {
                scope: scope(r)?,
                name: name(r)?,
            },
            value: expr(r)?,
        })
    })
}

fn write_expr_pairs(w: &mut CborWriter, values: &BTreeMap<String, Expr>) {
    write_pairs(w, values, write_name, write_expr);
}

fn expr_pairs(r: &mut CborReader<'_>) -> D<BTreeMap<String, Expr>> {
    pairs(r, MAX_ARGS, name, expr)
}

fn write_scalar_pairs(w: &mut CborWriter, values: &BTreeMap<String, Scalar>) {
    write_pairs(w, values, write_name, write_scalar);
}

fn scalar_pairs(r: &mut CborReader<'_>, max: usize) -> D<BTreeMap<String, Scalar>> {
    pairs(r, max, name, scalar)
}

// --- plans ---

fn write_optional_ref(w: &mut CborWriter, key: u64, value: &Option<ContentRef>) {
    if let Some(value) = value {
        w.unsigned(key);
        value.encode(w);
    }
}

fn write_option(w: &mut CborWriter, value: &OptionPlan) {
    map_header(
        w,
        &[
            true,
            value.label.is_some(),
            value.visible_if.is_some(),
            value.enabled_if.is_some(),
            value.reason.is_some(),
            true,
            true,
        ],
    );
    w.unsigned(0);
    w.bytes(value.id.as_bytes());
    write_optional_ref(w, 1, &value.label);
    if let Some(condition) = &value.visible_if {
        w.unsigned(2);
        write_expr(w, condition);
    }
    if let Some(condition) = &value.enabled_if {
        w.unsigned(3);
        write_expr(w, condition);
    }
    write_optional_ref(w, 4, &value.reason);
    w.unsigned(5);
    write_assignments(w, &value.effects);
    w.unsigned(6);
    match &value.outcome {
        Outcome::Local { reply, rejoin } => {
            w.array(3);
            w.unsigned(0);
            match reply {
                Some(reply) => reply.encode(w),
                None => w.null(),
            }
            match rejoin {
                Some(rejoin) => w.text(rejoin.as_str()),
                None => w.null(),
            }
        }
        Outcome::Branch { target } => {
            w.array(2);
            w.unsigned(1);
            w.bytes(target.as_bytes());
        }
    }
}

fn option_plan(r: &mut CborReader<'_>) -> D<OptionPlan> {
    let (mut id, mut label, mut visible_if, mut enabled_if, mut reason) =
        (None, None, None, None, None);
    let (mut effects, mut outcome) = (None, None);
    let mut map = MapIn::new(r, 7)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => id = Some(option_id(r)?),
            1 => label = Some(ContentRef::decode(r)?),
            2 => visible_if = Some(expr(r)?),
            3 => enabled_if = Some(expr(r)?),
            4 => reason = Some(ContentRef::decode(r)?),
            5 => effects = Some(assignments(r)?),
            6 => {
                let length = r.array_len()?;
                outcome = Some(match (r.unsigned()?, length) {
                    (0, 3) => Outcome::Local {
                        reply: r.optional(Segment::decode)?,
                        rejoin: r.optional(anchor)?,
                    },
                    (1, 2) => Outcome::Branch {
                        target: node_id(r)?,
                    },
                    _ => return Err(DecodeError::Schema("option outcome")),
                });
            }
            _ => return Err(unknown()),
        }
    }
    Ok(OptionPlan {
        id: required(id, "option id")?,
        label,
        visible_if,
        enabled_if,
        reason,
        effects: required(effects, "option effects")?,
        outcome: required(outcome, "option outcome")?,
    })
}

fn write_options(w: &mut CborWriter, options: &[OptionPlan]) {
    w.array(options.len() as u64);
    for option in options {
        write_option(w, option);
    }
}

fn write_choice_point(w: &mut CborWriter, value: &ChoicePoint) {
    map_header(
        w,
        &[
            true,
            value.placement.is_some(),
            true,
            true,
            true,
            value.proposals,
        ],
    );
    w.unsigned(0);
    w.bytes(value.id.as_bytes());
    if let Some(placement) = &value.placement {
        w.unsigned(1);
        w.text(placement.as_str());
    }
    w.unsigned(2);
    w.unsigned(u64::from(value.min));
    w.unsigned(3);
    w.unsigned(u64::from(value.max));
    w.unsigned(4);
    write_options(w, &value.options);
    // Absent means false, so choice points without proposals keep their bytes.
    if value.proposals {
        w.unsigned(5);
        w.boolean(true);
    }
}

fn choice_point(r: &mut CborReader<'_>) -> D<ChoicePoint> {
    let (mut id, mut placement, mut min, mut max, mut options) = (None, None, None, None, None);
    let mut proposals = false;
    let mut map = MapIn::new(r, 6)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => id = Some(choice_point_id(r)?),
            1 => placement = Some(anchor(r)?),
            2 => min = Some(u16_value(r)?),
            3 => max = Some(u16_value(r)?),
            4 => options = Some(array(r, MAX_OPTIONS, option_plan)?),
            5 => proposals = r.boolean()?,
            _ => return Err(unknown()),
        }
    }
    Ok(ChoicePoint {
        id: required(id, "choice point id")?,
        placement,
        min: required(min, "choice point min")?,
        max: required(max, "choice point max")?,
        options: required(options, "choice point options")?,
        proposals,
    })
}

fn write_passage(w: &mut CborWriter, value: &Passage) {
    map_header(
        w,
        &[
            value.title.is_some(),
            value.body.is_some(),
            true,
            true,
            value.next.is_some(),
        ],
    );
    write_optional_ref(w, 0, &value.title);
    if let Some(body) = &value.body {
        w.unsigned(1);
        body.encode(w);
    }
    w.unsigned(2);
    write_expr_pairs(w, &value.args);
    w.unsigned(3);
    w.array(value.choice_points.len() as u64);
    for point in &value.choice_points {
        write_choice_point(w, point);
    }
    if let Some(next) = &value.next {
        w.unsigned(4);
        w.bytes(next.as_bytes());
    }
}

fn passage(r: &mut CborReader<'_>) -> D<Passage> {
    let (mut title, mut body, mut args, mut points, mut next) = (None, None, None, None, None);
    let mut map = MapIn::new(r, 5)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => title = Some(ContentRef::decode(r)?),
            1 => body = Some(Segment::decode(r)?),
            2 => args = Some(expr_pairs(r)?),
            3 => points = Some(array(r, MAX_CHOICE_POINTS, choice_point)?),
            4 => next = Some(node_id(r)?),
            _ => return Err(unknown()),
        }
    }
    Ok(Passage {
        title,
        body,
        args: required(args, "passage args")?,
        choice_points: required(points, "passage choice points")?,
        next,
    })
}

fn write_plan(w: &mut CborWriter, value: &Plan) {
    match value {
        Plan::Passage(value) => {
            w.array(2);
            w.unsigned(0);
            write_passage(w, value);
        }
        Plan::Branch {
            condition,
            when_true,
            when_false,
        } => {
            w.array(4);
            w.unsigned(1);
            write_expr(w, condition);
            w.bytes(when_true.as_bytes());
            w.bytes(when_false.as_bytes());
        }
        Plan::Mutate { assignments, next } => {
            w.array(3);
            w.unsigned(2);
            write_assignments(w, assignments);
            w.bytes(next.as_bytes());
        }
        Plan::Call {
            target,
            arguments,
            on_return,
        } => {
            w.array(4);
            w.unsigned(3);
            w.array(2);
            match target {
                CallTarget::Local { graph } => {
                    w.unsigned(0);
                    w.text(graph);
                }
                CallTarget::Import { port } => {
                    w.unsigned(1);
                    w.text(port);
                }
            }
            write_expr_pairs(w, arguments);
            write_pairs(w, on_return, write_name, |w, node: &NodeId| {
                w.bytes(node.as_bytes())
            });
        }
        Plan::Return { outcome } => {
            w.array(2);
            w.unsigned(4);
            w.text(outcome);
        }
    }
}

fn plan(r: &mut CborReader<'_>) -> D<Plan> {
    let length = r.array_len()?;
    match (r.unsigned()?, length) {
        (0, 2) => Ok(Plan::Passage(passage(r)?)),
        (1, 4) => Ok(Plan::Branch {
            condition: expr(r)?,
            when_true: node_id(r)?,
            when_false: node_id(r)?,
        }),
        (2, 3) => Ok(Plan::Mutate {
            assignments: assignments(r)?,
            next: node_id(r)?,
        }),
        (3, 4) => {
            exact_array(r, 2)?;
            let target = match r.unsigned()? {
                0 => CallTarget::Local { graph: name(r)? },
                1 => CallTarget::Import { port: name(r)? },
                _ => return Err(DecodeError::Schema("call target")),
            };
            Ok(Plan::Call {
                target,
                arguments: expr_pairs(r)?,
                on_return: pairs(r, MAX_VARIABLES, name, node_id)?,
            })
        }
        (4, 2) => Ok(Plan::Return { outcome: name(r)? }),
        _ => Err(DecodeError::Schema("node plan")),
    }
}

// --- chunks ---

fn write_graph(w: &mut CborWriter, value: &Graph) {
    let header = &value.header;
    map_header(
        w,
        &[
            header.title.is_some(),
            true,
            true,
            true,
            true,
            true,
            true,
            true,
        ],
    );
    write_optional_ref(w, 0, &header.title);
    w.unsigned(1);
    write_pairs(w, &header.parameters, write_name, write_scalar_type);
    w.unsigned(2);
    write_scalar_pairs(w, &header.locals);
    w.unsigned(3);
    write_pairs(w, &header.shared, write_name, write_scalar_type);
    w.unsigned(4);
    write_pairs(w, &header.imports, write_name, write_signature);
    w.unsigned(5);
    write_outcomes(w, &header.outcomes);
    w.unsigned(6);
    w.bytes(header.entry.as_bytes());
    w.unsigned(7);
    write_pairs(
        w,
        &value.nodes,
        |w, id: &NodeId| w.bytes(id.as_bytes()),
        write_plan,
    );
}

fn graph(r: &mut CborReader<'_>) -> D<Graph> {
    let mut title = None;
    let (mut parameters, mut locals, mut shared, mut imports) = (None, None, None, None);
    let (mut outcome_names, mut entry, mut nodes) = (None, None, None);
    let mut map = MapIn::new(r, 8)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => title = Some(ContentRef::decode(r)?),
            1 => parameters = Some(pairs(r, MAX_VARIABLES, name, scalar_type)?),
            2 => locals = Some(scalar_pairs(r, MAX_VARIABLES)?),
            3 => shared = Some(pairs(r, MAX_SHARED, name, scalar_type)?),
            4 => imports = Some(pairs(r, MAX_VARIABLES, name, signature)?),
            5 => outcome_names = Some(outcomes(r)?),
            6 => entry = Some(node_id(r)?),
            7 => nodes = Some(pairs(r, MAX_GRAPH_NODES, node_id, plan)?),
            _ => return Err(unknown()),
        }
    }
    Ok(Graph {
        header: GraphHeader {
            title,
            parameters: required(parameters, "graph parameters")?,
            locals: required(locals, "graph locals")?,
            shared: required(shared, "graph shared")?,
            imports: required(imports, "graph imports")?,
            outcomes: required(outcome_names, "graph outcomes")?,
            entry: required(entry, "graph entry")?,
        },
        nodes: required(nodes, "graph nodes")?,
    })
}

pub(crate) fn encode_chunk(value: &Chunk) -> Vec<u8> {
    let mut w = CborWriter::new();
    w.map(1);
    w.unsigned(0);
    write_pairs(&mut w, &value.graphs, write_graph_ref, write_graph);
    w.into_bytes()
}

pub(crate) fn decode_chunk(r: &mut CborReader<'_>) -> D<Chunk> {
    let mut graphs = None;
    let mut map = MapIn::new(r, 1)?;
    while let Some(key) = map.next()? {
        match key {
            0 => graphs = Some(pairs(map.r(), 1 << 16, graph_ref, graph)?),
            _ => return Err(unknown()),
        }
    }
    Ok(Chunk {
        graphs: required(graphs, "chunk graphs")?,
    })
}

// --- manifest ---

fn write_product(w: &mut CborWriter, value: &ProductHeader) {
    map_header(
        w,
        &[true, value.title.is_some(), true, true, true, true, true],
    );
    w.unsigned(0);
    w.text(&value.id);
    write_optional_ref(w, 1, &value.title);
    w.unsigned(2);
    write_graph_ref(w, &value.entry);
    w.unsigned(3);
    write_scalar_pairs(w, &value.arguments);
    w.unsigned(4);
    write_pairs(
        w,
        &value.shared,
        write_name,
        |w, variable: &SharedVariable| {
            w.map(1 + u64::from(variable.label.is_some()));
            w.unsigned(0);
            write_scalar(w, &variable.value);
            write_optional_ref(w, 1, &variable.label);
        },
    );
    w.unsigned(5);
    write_pairs(w, &value.endings, write_name, |w, ending: &Ending| {
        w.map(u64::from(ending.title.is_some()) + u64::from(ending.body.is_some()));
        write_optional_ref(w, 0, &ending.title);
        if let Some(body) = &ending.body {
            w.unsigned(1);
            body.encode(w);
        }
    });
    w.unsigned(6);
    write_pairs(w, &value.bindings, write_import_ref, write_graph_ref);
}

fn shared_variable(r: &mut CborReader<'_>) -> D<SharedVariable> {
    let (mut value, mut label) = (None, None);
    let mut map = MapIn::new(r, 2)?;
    while let Some(key) = map.next()? {
        match key {
            0 => value = Some(scalar(map.r())?),
            1 => label = Some(ContentRef::decode(map.r())?),
            _ => return Err(unknown()),
        }
    }
    Ok(SharedVariable {
        value: required(value, "shared value")?,
        label,
    })
}

fn ending(r: &mut CborReader<'_>) -> D<Ending> {
    let mut value = Ending::default();
    let mut map = MapIn::new(r, 2)?;
    while let Some(key) = map.next()? {
        match key {
            0 => value.title = Some(ContentRef::decode(map.r())?),
            1 => value.body = Some(Segment::decode(map.r())?),
            _ => return Err(unknown()),
        }
    }
    Ok(value)
}

fn product(r: &mut CborReader<'_>) -> D<ProductHeader> {
    let (mut id, mut title, mut entry, mut arguments) = (None, None, None, None);
    let (mut shared, mut endings, mut bindings) = (None, None, None);
    let mut map = MapIn::new(r, 7)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => id = Some(name(r)?),
            1 => title = Some(ContentRef::decode(r)?),
            2 => entry = Some(graph_ref(r)?),
            3 => arguments = Some(scalar_pairs(r, MAX_VARIABLES)?),
            4 => shared = Some(pairs(r, MAX_SHARED, name, shared_variable)?),
            5 => endings = Some(pairs(r, MAX_VARIABLES, name, ending)?),
            6 => bindings = Some(pairs(r, 1 << 16, import_ref, graph_ref)?),
            _ => return Err(unknown()),
        }
    }
    Ok(ProductHeader {
        id: required(id, "product id")?,
        title,
        entry: required(entry, "product entry")?,
        arguments: required(arguments, "product arguments")?,
        shared: required(shared, "product shared")?,
        endings: required(endings, "product endings")?,
        bindings: required(bindings, "product bindings")?,
    })
}

pub(crate) fn encode_manifest(value: &Manifest) -> Vec<u8> {
    let mut w = CborWriter::new();
    w.map(6);
    w.unsigned(0);
    write_product(&mut w, &value.product);
    w.unsigned(1);
    write_pairs(&mut w, &value.packages, write_name, |w, package| {
        w.array(2);
        w.text(&package.id);
        w.text(&package.version);
    });
    w.unsigned(2);
    write_pairs(&mut w, &value.graphs, write_graph_ref, |w, entry| {
        w.map(3);
        w.unsigned(0);
        write_signature(w, &entry.signature);
        w.unsigned(1);
        w.boolean(entry.exported);
        w.unsigned(2);
        w.unsigned(u64::from(entry.chunk));
    });
    w.unsigned(3);
    w.array(value.chunks.len() as u64);
    for chunk in &value.chunks {
        w.bytes(chunk.as_bytes());
    }
    w.unsigned(4);
    write_pairs(&mut w, &value.node_types, write_name, write_name);
    w.unsigned(5);
    w.bytes(value.tombstones.as_bytes());
    w.into_bytes()
}

fn graph_entry(r: &mut CborReader<'_>) -> D<GraphEntry> {
    let (mut signature_value, mut exported, mut chunk) = (None, None, None);
    let mut map = MapIn::new(r, 3)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => signature_value = Some(signature(r)?),
            1 => exported = Some(r.boolean()?),
            2 => chunk = Some(u32_value(r)?),
            _ => return Err(unknown()),
        }
    }
    Ok(GraphEntry {
        signature: required(signature_value, "graph signature")?,
        exported: required(exported, "graph exported")?,
        chunk: required(chunk, "graph chunk")?,
    })
}

fn type_id(r: &mut CborReader<'_>) -> D<String> {
    let text = r.text(MAX_TYPE_ID_BYTES)?;
    if crate::registry::valid_type_id(text) {
        Ok(text.to_owned())
    } else {
        Err(DecodeError::Schema("node type id"))
    }
}

pub(crate) fn decode_manifest(r: &mut CborReader<'_>) -> D<Manifest> {
    let (mut product_value, mut packages, mut graphs) = (None, None, None);
    let (mut chunks, mut node_types, mut tombstones) = (None, None, None);
    let mut map = MapIn::new(r, 6)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => product_value = Some(product(r)?),
            1 => {
                packages = Some(pairs(r, 1 << 16, name, |r| {
                    exact_array(r, 2)?;
                    Ok(PackageInstance {
                        id: bounded_text(r, MAX_PACKAGE_ID_BYTES)?,
                        version: bounded_text(r, MAX_VERSION_BYTES)?,
                    })
                })?)
            }
            2 => graphs = Some(pairs(r, 1 << 20, graph_ref, graph_entry)?),
            3 => chunks = Some(array(r, 1 << 20, object)?),
            4 => {
                node_types = Some(pairs(r, 1 << 10, type_id, |r| {
                    bounded_text(r, MAX_REVISION_BYTES)
                })?)
            }
            5 => tombstones = Some(object(r)?),
            _ => return Err(unknown()),
        }
    }
    Ok(Manifest {
        product: required(product_value, "manifest product")?,
        packages: required(packages, "manifest packages")?,
        graphs: required(graphs, "manifest graphs")?,
        chunks: required(chunks, "manifest chunks")?,
        node_types: required(node_types, "manifest node types")?,
        tombstones: required(tombstones, "manifest tombstones")?,
    })
}

// --- tombstones and names ---

pub(crate) fn encode_tombstones(value: &TombstoneSet) -> Vec<u8> {
    let mut w = CborWriter::new();
    w.map(3);
    w.unsigned(0);
    w.array(value.nodes.len() as u64);
    for id in &value.nodes {
        w.bytes(id.as_bytes());
    }
    w.unsigned(1);
    w.array(value.choice_points.len() as u64);
    for id in &value.choice_points {
        w.bytes(id.as_bytes());
    }
    w.unsigned(2);
    w.array(value.options.len() as u64);
    for id in &value.options {
        w.bytes(id.as_bytes());
    }
    w.into_bytes()
}

pub(crate) fn decode_tombstones(r: &mut CborReader<'_>) -> D<TombstoneSet> {
    let (mut nodes, mut choice_points, mut options) = (None, None, None);
    let mut map = MapIn::new(r, 3)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => nodes = Some(sorted_set(r, 1 << 22, node_id)?),
            1 => choice_points = Some(sorted_set(r, 1 << 22, choice_point_id)?),
            2 => options = Some(sorted_set(r, 1 << 22, option_id)?),
            _ => return Err(unknown()),
        }
    }
    Ok(TombstoneSet {
        nodes: required(nodes, "tombstone nodes")?,
        choice_points: required(choice_points, "tombstone choice points")?,
        options: required(options, "tombstone options")?,
    })
}

pub(crate) fn encode_names(value: &NameTable) -> Vec<u8> {
    let mut w = CborWriter::new();
    w.map(2 + u64::from(value.migrated_from_r1.is_some()));
    w.unsigned(0);
    w.bytes(value.artifact.as_bytes());
    if let Some(r1) = &value.migrated_from_r1 {
        w.unsigned(1);
        w.bytes(&r1.0);
    }
    w.unsigned(2);
    write_pairs(&mut w, &value.graphs, write_graph_ref, |w, names| {
        w.map(3);
        w.unsigned(0);
        write_pairs(
            w,
            &names.nodes,
            |w, id: &NodeId| w.bytes(id.as_bytes()),
            write_name,
        );
        w.unsigned(1);
        write_pairs(
            w,
            &names.choice_points,
            |w, id: &ChoicePointId| w.bytes(id.as_bytes()),
            write_name,
        );
        w.unsigned(2);
        write_pairs(
            w,
            &names.options,
            |w, id: &OptionId| w.bytes(id.as_bytes()),
            write_name,
        );
    });
    w.into_bytes()
}

pub(crate) fn decode_names(r: &mut CborReader<'_>) -> D<NameTable> {
    let (mut artifact, mut r1, mut graphs) = (None, None, None);
    let mut map = MapIn::new(r, 3)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => artifact = Some(ArtifactId::from_bytes(r.bytes_exact::<32>()?)),
            1 => r1 = Some(R1ArtifactId(r.bytes_exact::<32>()?)),
            2 => {
                graphs = Some(pairs(r, 1 << 20, graph_ref, |r| {
                    let mut names = GraphNames::default();
                    let mut map = MapIn::new(r, 3)?;
                    while let Some(key) = map.next()? {
                        let r = map.r();
                        match key {
                            0 => names.nodes = pairs(r, MAX_GRAPH_NODES, node_id, name)?,
                            1 => names.choice_points = pairs(r, 1 << 18, choice_point_id, name)?,
                            2 => names.options = pairs(r, 1 << 22, option_id, name)?,
                            _ => return Err(unknown()),
                        }
                    }
                    Ok(names)
                })?)
            }
            _ => return Err(unknown()),
        }
    }
    Ok(NameTable {
        artifact: required(artifact, "names artifact")?,
        migrated_from_r1: r1,
        graphs: required(graphs, "names graphs")?,
    })
}

// --- pack ---

/// Single-file distribution (ADR 0013 §8): every object as its own envelope, so the bytes
/// equal what a kernel store keeps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pack {
    pub manifest: Vec<u8>,
    pub chunks: Vec<Vec<u8>>,
    pub tombstones: Vec<u8>,
    pub names: Option<Vec<u8>>,
}

impl Pack {
    pub fn encode(&self) -> Vec<u8> {
        encode_envelope(KIND_PACK, SCHEMA, &self.payload())
    }

    fn payload(&self) -> Vec<u8> {
        let mut w = CborWriter::new();
        w.map(3 + u64::from(self.names.is_some()));
        w.unsigned(0);
        w.bytes(&self.manifest);
        w.unsigned(1);
        w.array(self.chunks.len() as u64);
        for chunk in &self.chunks {
            w.bytes(chunk);
        }
        w.unsigned(2);
        w.bytes(&self.tombstones);
        if let Some(names) = &self.names {
            w.unsigned(3);
            w.bytes(names);
        }
        w.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        open(
            bytes,
            KIND_PACK,
            crate::MAX_PACK_BYTES,
            "pack",
            |r| {
                let (mut manifest, mut chunks, mut tombstones, mut names) =
                    (None, None, None, None);
                let mut map = MapIn::new(r, 4)?;
                let max = crate::MAX_CHUNK_BYTES as u64 + 56;
                while let Some(key) = map.next()? {
                    let r = map.r();
                    match key {
                        0 => manifest = Some(r.bytes(max)?.to_vec()),
                        1 => chunks = Some(array(r, 1 << 20, |r| Ok(r.bytes(max)?.to_vec()))?),
                        2 => tombstones = Some(r.bytes(crate::MAX_PACK_BYTES as u64)?.to_vec()),
                        3 => names = Some(r.bytes(crate::MAX_PACK_BYTES as u64)?.to_vec()),
                        _ => return Err(unknown()),
                    }
                }
                Ok(Pack {
                    manifest: required(manifest, "pack manifest")?,
                    chunks: required(chunks, "pack chunks")?,
                    tombstones: required(tombstones, "pack tombstones")?,
                    names,
                })
            },
            Pack::payload,
        )
        .map(|(pack, _)| pack)
    }
}

// --- session objects ---

pub(crate) fn encode_state(value: &State) -> Vec<u8> {
    let mut w = CborWriter::new();
    w.map(3 + u64::from(value.finished.is_some()));
    w.unsigned(0);
    write_scalar_pairs(&mut w, &value.shared);
    w.unsigned(1);
    w.array(value.frames.len() as u64);
    for frame in &value.frames {
        map_header(
            &mut w,
            &[
                true,
                true,
                frame.at.is_some(),
                true,
                true,
                true,
                !frame.overlay.is_empty(),
            ],
        );
        w.unsigned(0);
        write_graph_ref(&mut w, &frame.graph);
        w.unsigned(1);
        w.bytes(frame.node.as_bytes());
        if let Some(at) = &frame.at {
            w.unsigned(2);
            w.bytes(at.as_bytes());
        }
        w.unsigned(3);
        w.unsigned(u64::from(frame.instance));
        w.unsigned(4);
        write_scalar_pairs(&mut w, &frame.parameters);
        w.unsigned(5);
        write_scalar_pairs(&mut w, &frame.locals);
        if !frame.overlay.is_empty() {
            w.unsigned(6);
            write_overlay(&mut w, &frame.overlay);
        }
    }
    w.unsigned(2);
    w.unsigned(u64::from(value.next_instance));
    if let Some(finished) = &value.finished {
        w.unsigned(3);
        w.map(3);
        w.unsigned(0);
        w.bytes(finished.node.as_bytes());
        w.unsigned(1);
        w.unsigned(u64::from(finished.instance));
        w.unsigned(2);
        w.text(&finished.outcome);
    }
    w.into_bytes()
}

/// Absent fields are empty, and an empty overlay is absent from its frame.
fn write_overlay(w: &mut CborWriter, value: &Overlay) {
    map_header(w, &[!value.nodes.is_empty(), !value.options.is_empty()]);
    if !value.nodes.is_empty() {
        w.unsigned(0);
        write_pairs(
            w,
            &value.nodes,
            |w, id: &NodeId| w.bytes(id.as_bytes()),
            write_passage,
        );
    }
    if !value.options.is_empty() {
        w.unsigned(1);
        write_pairs(
            w,
            &value.options,
            |w, id: &ChoicePointId| w.bytes(id.as_bytes()),
            |w, options: &Vec<OptionPlan>| write_options(w, options),
        );
    }
}

fn overlay(r: &mut CborReader<'_>) -> D<Overlay> {
    let mut value = Overlay::default();
    let mut map = MapIn::new(r, 2)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => value.nodes = pairs(r, MAX_OVERLAY_NODES, node_id, passage)?,
            1 => {
                value.options = pairs(r, 1 << 16, choice_point_id, |r| {
                    let options = array(r, MAX_OPTIONS, option_plan)?;
                    if options.is_empty() {
                        return Err(DecodeError::Schema("empty overlay options"));
                    }
                    Ok(options)
                })?
            }
            _ => return Err(unknown()),
        }
    }
    Ok(value)
}

fn frame(r: &mut CborReader<'_>) -> D<Frame> {
    let (mut graph_value, mut node, mut at, mut instance) = (None, None, None, None);
    let (mut parameters, mut locals, mut overlay_value) = (None, None, None);
    let mut map = MapIn::new(r, 7)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => graph_value = Some(graph_ref(r)?),
            1 => node = Some(node_id(r)?),
            2 => at = Some(choice_point_id(r)?),
            3 => instance = Some(u32_value(r)?),
            4 => parameters = Some(scalar_pairs(r, MAX_VARIABLES)?),
            5 => locals = Some(scalar_pairs(r, MAX_VARIABLES)?),
            6 => overlay_value = Some(overlay(r)?),
            _ => return Err(unknown()),
        }
    }
    Ok(Frame {
        graph: required(graph_value, "frame graph")?,
        node: required(node, "frame node")?,
        at,
        instance: required(instance, "frame instance")?,
        parameters: required(parameters, "frame parameters")?,
        locals: required(locals, "frame locals")?,
        overlay: overlay_value.unwrap_or_default(),
    })
}

pub(crate) fn decode_state(r: &mut CborReader<'_>) -> D<State> {
    let (mut shared, mut frames, mut next_instance, mut finished) = (None, None, None, None);
    let mut map = MapIn::new(r, 4)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => shared = Some(scalar_pairs(r, MAX_SHARED)?),
            1 => frames = Some(array(r, MAX_CALL_DEPTH, frame)?),
            2 => next_instance = Some(u32_value(r)?),
            3 => {
                let (mut node, mut instance, mut outcome) = (None, None, None);
                let mut map = MapIn::new(r, 3)?;
                while let Some(key) = map.next()? {
                    let r = map.r();
                    match key {
                        0 => node = Some(node_id(r)?),
                        1 => instance = Some(u32_value(r)?),
                        2 => outcome = Some(name(r)?),
                        _ => return Err(unknown()),
                    }
                }
                finished = Some(Finished {
                    node: required(node, "finished node")?,
                    instance: required(instance, "finished instance")?,
                    outcome: required(outcome, "finished outcome")?,
                });
            }
            _ => return Err(unknown()),
        }
    }
    Ok(State {
        shared: required(shared, "state shared")?,
        frames: required(frames, "state frames")?,
        next_instance: required(next_instance, "state next instance")?,
        finished,
    })
}

pub(crate) fn encode_input(value: &Input) -> Vec<u8> {
    let mut w = CborWriter::new();
    match value {
        Input::Choose {
            choice_point,
            options,
        } => {
            w.array(3);
            w.unsigned(0);
            w.bytes(choice_point.as_bytes());
            w.array(options.len() as u64);
            for option in options {
                w.bytes(option.as_bytes());
            }
        }
        Input::Propose {
            choice_point,
            options,
            nodes,
        } => {
            w.array(4);
            w.unsigned(1);
            w.bytes(choice_point.as_bytes());
            write_options(&mut w, options);
            w.array(nodes.len() as u64);
            for (id, passage) in nodes {
                w.array(2);
                w.bytes(id.as_bytes());
                write_passage(&mut w, passage);
            }
        }
    }
    w.into_bytes()
}

pub(crate) fn decode_input(r: &mut CborReader<'_>) -> D<Input> {
    let length = r.array_len()?;
    match (r.unsigned()?, length) {
        (0, 3) => Ok(Input::Choose {
            choice_point: choice_point_id(r)?,
            options: array(r, MAX_OPTIONS, option_id)?,
        }),
        (1, 4) => Ok(Input::Propose {
            choice_point: choice_point_id(r)?,
            options: array(r, MAX_PROPOSED_OPTIONS, option_plan)?,
            nodes: array(r, MAX_PROPOSED_NODES, |r| {
                exact_array(r, 2)?;
                Ok((node_id(r)?, passage(r)?))
            })?,
        }),
        _ => Err(DecodeError::Schema("input")),
    }
}

pub(crate) fn encode_commit(value: &Commit) -> Vec<u8> {
    let mut w = CborWriter::new();
    map_header(
        &mut w,
        &[
            true,
            value.parent.is_some(),
            value.input.is_some(),
            true,
            true,
        ],
    );
    w.unsigned(0);
    w.bytes(value.artifact.as_bytes());
    if let Some(parent) = &value.parent {
        w.unsigned(1);
        w.bytes(parent.as_bytes());
    }
    if let Some(input) = &value.input {
        w.unsigned(2);
        w.bytes(input.as_bytes());
    }
    w.unsigned(3);
    w.bytes(value.state.as_bytes());
    w.unsigned(4);
    w.unsigned(value.depth);
    w.into_bytes()
}

pub(crate) fn decode_commit(r: &mut CborReader<'_>) -> D<Commit> {
    let (mut artifact, mut parent, mut input, mut state, mut depth) =
        (None, None, None, None, None);
    let mut map = MapIn::new(r, 5)?;
    while let Some(key) = map.next()? {
        let r = map.r();
        match key {
            0 => artifact = Some(ArtifactId::from_bytes(r.bytes_exact::<32>()?)),
            1 => parent = Some(CommitId::from_bytes(r.bytes_exact::<32>()?)),
            2 => input = Some(object(r)?),
            3 => state = Some(object(r)?),
            4 => depth = Some(r.unsigned()?),
            _ => return Err(unknown()),
        }
    }
    Ok(Commit {
        artifact: required(artifact, "commit artifact")?,
        parent,
        input,
        state: required(state, "commit state")?,
        depth: required(depth, "commit depth")?,
    })
}

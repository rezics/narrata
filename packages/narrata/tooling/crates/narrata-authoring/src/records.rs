//! Draft record v1 is a reversible split of R2 source, not a new execution vocabulary.

use std::collections::{BTreeMap, BTreeSet};

use narrata_kernel::content::{AnchorId, ContentRef};
use narrata_nodes::{
    AuthoredId, ChoicePointId, Error, NodeId, OptionId, PackageSource, ProjectManifest,
    ProjectSource, Result, SourcePlan, canonical_json, lower_builtin,
    source::{ChoicePointSource, GraphSource, NodeSource, OutcomeSource},
};
use schemars::{JsonSchema, Schema, json_schema, schema_for};
use serde::{Deserialize, Serialize};

use crate::validate::{Collector, Diagnostic, Location, Report, Severity, source_pointer};

pub const MAX_RECORD_BYTES: usize = 1024 * 1024;

/// Only v1 can be constructed or decoded, so a typed record cannot accidentally write a
/// future version using today's splitting rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DraftVersion;

impl Serialize for DraftVersion {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_u16(1)
    }
}
impl<'de> Deserialize<'de> for DraftVersion {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        if u16::deserialize(deserializer)? == 1 {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom("unsupported draft record version"))
        }
    }
}
impl JsonSchema for DraftVersion {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DraftVersion".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> Schema {
        json_schema!({"type": "integer", "const": 1})
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DraftRecord {
    Project {
        format_version: DraftVersion,
        payload: ProjectManifest,
    },
    Package {
        format_version: DraftVersion,
        package: String,
        payload: PackageSource,
    },
    Graph {
        format_version: DraftVersion,
        package: String,
        graph: String,
        payload: GraphSource,
    },
    Node {
        format_version: DraftVersion,
        package: String,
        graph: String,
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        choice_order: Option<Vec<ChoicePointId>>,
        payload: NodeSource,
    },
    ChoicePoint {
        format_version: DraftVersion,
        package: String,
        graph: String,
        node: NodeId,
        payload: ChoicePointSource,
    },
    Tombstone {
        format_version: DraftVersion,
        package: String,
        payload: AuthoredId,
    },
}

#[derive(Clone, Debug)]
pub struct CheckedRecord {
    record: DraftRecord,
    bytes: Vec<u8>,
    work: usize,
}

impl CheckedRecord {
    pub fn record(&self) -> &DraftRecord {
        &self.record
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn key(&self) -> String {
        self.record.key()
    }
    pub(crate) fn work(&self) -> usize {
        self.work
    }

    pub fn new(record: DraftRecord) -> Report<Self> {
        let mut out = Collector::new(100_000);
        match Self::from_checked(record) {
            Ok(record) => {
                record.record.check(&mut out);
                out.finish(Some(record))
            }
            Err(error) => {
                out.error(error, None);
                out.finish(None)
            }
        }
    }

    pub(crate) fn from_checked(record: DraftRecord) -> Result<Self> {
        if let DraftRecord::Node { payload, .. } = &record {
            narrata_nodes::check_node_data(&payload.data, &record.source_path())?;
        }
        let bytes = canonical_json(&record)?;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(Error::new(
                "record_too_large",
                record.source_path(),
                "draft record exceeds 1 MiB",
            ));
        }
        let value = serde_json::to_value(&record)
            .map_err(|error| Error::new("encoding", record.source_path(), error.to_string()))?;
        let work = crate::validate::value_work(&value, usize::MAX).unwrap_or(usize::MAX);
        Ok(Self {
            record,
            bytes,
            work,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CheckedRecords {
    records: Vec<CheckedRecord>,
    source: ProjectSource,
}

impl CheckedRecords {
    pub fn new(records: Vec<CheckedRecord>) -> Report<Self> {
        let assembled = assemble_records(&records);
        Report {
            complete: assembled.complete,
            diagnostics: assembled.diagnostics,
            value: assembled.value.map(|source| Self { records, source }),
        }
    }
    pub fn records(&self) -> &[CheckedRecord] {
        &self.records
    }
    pub fn source(&self) -> &ProjectSource {
        &self.source
    }
    pub(crate) fn from_assembled(records: Vec<CheckedRecord>, source: ProjectSource) -> Self {
        Self { records, source }
    }
}

fn valid_alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= narrata_nodes::MAX_NAME_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

impl DraftRecord {
    pub fn key(&self) -> String {
        match self {
            Self::Project { .. } => "project".into(),
            Self::Package { package, .. } => format!("package:{package}"),
            Self::Graph { package, graph, .. } => format!("graph:{package}/{graph}"),
            Self::Node {
                payload,
                package,
                graph,
                key,
                ..
            } => payload
                .id
                .map(|id| id.to_string())
                .unwrap_or_else(|| format!("node:{package}/{graph}/{key}")),
            Self::ChoicePoint { payload, .. } => payload
                .id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "choice-point:unminted".into()),
            Self::Tombstone {
                package, payload, ..
            } => format!("tombstone:{package}/{payload}"),
        }
    }

    pub fn source_path(&self) -> String {
        match self {
            Self::Project { .. } => String::new(),
            Self::Package { package, .. } => format!("packages.{package}"),
            Self::Graph { package, graph, .. } => format!("packages.{package}.graphs.{graph}"),
            Self::Node {
                package,
                graph,
                key,
                ..
            } => format!("packages.{package}.graphs.{graph}.nodes.{key}"),
            Self::ChoicePoint {
                package,
                graph,
                node,
                ..
            } => format!("packages.{package}.graphs.{graph}.nodes.{node}.choice_points"),
            Self::Tombstone { package, .. } => format!("packages.{package}.tombstones"),
        }
    }

    pub(crate) fn location(&self, path: &str) -> Location {
        let mut location = Location {
            record: Some(self.key()),
            pointer: source_pointer(path),
            source_path: Some(self.source_path()),
            ..Location::default()
        };
        match self {
            Self::Node { payload, .. } => location.node = payload.id,
            Self::ChoicePoint { node, payload, .. } => {
                location.node = Some(*node);
                location.choice_point = payload.id;
                location.source_path = None;
                location.anchor = payload.placement.clone();
                if let Some(index) = path
                    .strip_prefix("/payload/options/")
                    .and_then(|tail| tail.split('/').next())
                    .and_then(|index| index.parse::<usize>().ok())
                    && let Some(option) = payload.options.get(index)
                {
                    location.option = option.id;
                }
            }
            _ => {}
        }
        location
    }

    pub(crate) fn check(&self, out: &mut Collector) {
        let error = |out: &mut Collector, code, pointer, message| {
            out.error(Error::new(code, pointer, message), Some(self))
        };
        let aliases: Vec<(&str, &str)> = match self {
            Self::Project { .. } => Vec::new(),
            Self::Package { package, .. } | Self::Tombstone { package, .. } => {
                vec![("/package", package)]
            }
            Self::Graph { package, graph, .. } | Self::ChoicePoint { package, graph, .. } => {
                vec![("/package", package), ("/graph", graph)]
            }
            Self::Node {
                package,
                graph,
                key,
                ..
            } => vec![("/package", package), ("/graph", graph), ("/key", key)],
        };
        for (pointer, alias) in aliases {
            if !valid_alias(alias) {
                error(out, "identifier", pointer, "invalid source alias");
            }
        }
        match self {
            Self::Project { payload, .. } => {
                for (key, value) in &payload.product.arguments {
                    if let Err(e) = narrata_nodes::check_scalar(
                        value,
                        &format!(
                            "/payload/product/arguments/{}",
                            crate::validate::escape(key)
                        ),
                    ) {
                        out.error(e, Some(self));
                    }
                }
                for e in narrata_nodes::validate_manifest_source(payload) {
                    out.error(
                        Error::new(
                            &e.code,
                            format!("/payload{}", source_pointer(&e.path)),
                            e.message,
                        ),
                        Some(self),
                    );
                }
                for alias in payload.packages.keys() {
                    if !valid_alias(alias) {
                        error(
                            out,
                            "identifier",
                            "/payload/packages",
                            "invalid package alias",
                        );
                    }
                }
            }
            Self::Package { payload, .. } => {
                if let Err(error) =
                    narrata_nodes::check_package_identity(&payload.id, &payload.version, "/payload")
                {
                    out.error(error, Some(self));
                }
                if !payload.graphs.is_empty() {
                    error(
                        out,
                        "record_shape",
                        "/payload/graphs",
                        "package header must have empty graphs",
                    );
                }
                if !payload.tombstones.is_empty() {
                    error(
                        out,
                        "record_shape",
                        "/payload/tombstones",
                        "tombstones have separate records",
                    );
                }
            }
            Self::Graph { payload, .. } => {
                for e in narrata_nodes::validate_graph_source(payload) {
                    out.error(
                        Error::new(
                            &e.code,
                            format!("/payload{}", source_pointer(&e.path)),
                            e.message,
                        ),
                        Some(self),
                    );
                }
                if !payload.nodes.is_empty() {
                    error(
                        out,
                        "record_shape",
                        "/payload/nodes",
                        "graph header must have empty nodes",
                    );
                }
            }
            Self::Node {
                choice_order,
                payload,
                ..
            } => {
                if payload.id.is_none() {
                    error(
                        out,
                        "missing_id",
                        "/payload/id",
                        "persisted node needs an ID",
                    );
                }
                if let Some(kind) = builtin_kind(&payload.type_id) {
                    match lower_builtin(kind, &payload.data) {
                        Ok(SourcePlan::Passage(passage)) => {
                            if payload.data.get("choice_points") != Some(&serde_json::json!([])) {
                                error(
                                    out,
                                    "record_shape",
                                    "/payload/data/choice_points",
                                    "passage header must have an empty choice_points array",
                                );
                            }
                            if !passage.choice_points.is_empty() {
                                error(
                                    out,
                                    "record_shape",
                                    "/payload/data/choice_points",
                                    "choice points have separate records",
                                );
                            }
                            match choice_order {
                                None => error(
                                    out,
                                    "record_shape",
                                    "/choice_order",
                                    "passage header needs choice_order",
                                ),
                                Some(order) => {
                                    if order.len() > narrata_nodes::MAX_CHOICE_POINTS {
                                        error(
                                            out,
                                            "limit",
                                            "/choice_order",
                                            "too many choice points",
                                        );
                                    }
                                    if order.iter().collect::<BTreeSet<_>>().len() != order.len() {
                                        error(
                                            out,
                                            "duplicate",
                                            "/choice_order",
                                            "choice_order contains duplicate IDs",
                                        );
                                    }
                                }
                            }
                        }
                        Ok(_) => {
                            if choice_order.is_some() {
                                error(
                                    out,
                                    "record_shape",
                                    "/choice_order",
                                    "control nodes have no choice_order",
                                );
                            }
                        }
                        Err(e) => {
                            out.error(Error::new(&e.code, "/payload/data", e.message), Some(self))
                        }
                    }
                } else if choice_order.is_some() {
                    error(
                        out,
                        "record_shape",
                        "/choice_order",
                        "only narrata.passage has choice_order",
                    );
                }
            }
            Self::ChoicePoint { payload, .. } => {
                for e in narrata_nodes::validate_choice_source(payload) {
                    out.error(e, Some(self));
                }
            }
            Self::Tombstone { .. } => {}
        }
    }
}

/// Normalize built-in data through its existing R2 type. This makes omitted defaults and
/// explicit defaults equivalent while leaving extension payloads opaque.
pub fn normalize_source(source: &ProjectSource) -> Result<ProjectSource> {
    let mut normalized = source.clone();
    for package in normalized.packages.values_mut() {
        for graph in package.graphs.values_mut() {
            for node in graph.nodes.values_mut() {
                if let Some(kind) = builtin_kind(&node.type_id) {
                    let plan = lower_builtin(kind, &node.data)?;
                    let mut data = serde_json::to_value(plan)
                        .map_err(|e| Error::new("json", "data", e.to_string()))?;
                    if let Some(fields) = data.as_object_mut() {
                        fields.remove("kind");
                    }
                    node.data = data;
                }
            }
        }
    }
    Ok(normalized)
}

fn builtin_kind(type_id: &str) -> Option<&str> {
    match type_id {
        "narrata.passage" | "narrata.branch" | "narrata.mutate" | "narrata.call"
        | "narrata.return" => type_id.strip_prefix("narrata."),
        _ => None,
    }
}

pub fn split_source(source: &ProjectSource) -> Report<Vec<CheckedRecord>> {
    let mut out = Collector::new(100_000);
    let source = match normalize_source(source) {
        Ok(source) => source,
        Err(error) => {
            out.error(error, None);
            return out.finish(None);
        }
    };
    let mut records = Vec::new();
    let mut add = |record: DraftRecord, out: &mut Collector| {
        let source_path = if let DraftRecord::ChoicePoint {
            package,
            graph,
            node,
            payload,
            ..
        } = &record
        {
            source.packages.get(package).and_then(|package| package.graphs.get(graph)).and_then(|source| source.nodes.iter().find(|(_, source)| source.id == Some(*node))).and_then(|(key, source)| {
                source.data.get("choice_points").and_then(serde_json::Value::as_array).and_then(|points| points.iter().position(|point| point.get("key").and_then(serde_json::Value::as_str) == Some(&payload.key))).map(|index| format!("packages.{package}.graphs.{graph}.nodes.{key}.choice_points[{index}]"))
            })
        } else {
            Some(record.source_path())
        };
        let report = CheckedRecord::new(record);
        for mut diagnostic in report.diagnostics {
            diagnostic.location.source_path = source_path.clone();
            out.push(diagnostic);
        }
        if let Some(record) = report.value {
            records.push(record);
        }
    };
    add(
        DraftRecord::Project {
            format_version: DraftVersion,
            payload: source.manifest.clone(),
        },
        &mut out,
    );
    for (package_alias, package) in &source.packages {
        let mut header = package.clone();
        header.graphs.clear();
        header.tombstones.clear();
        add(
            DraftRecord::Package {
                format_version: DraftVersion,
                package: package_alias.clone(),
                payload: header,
            },
            &mut out,
        );
        for (graph_alias, graph) in &package.graphs {
            let mut header = graph.clone();
            header.nodes.clear();
            add(
                DraftRecord::Graph {
                    format_version: DraftVersion,
                    package: package_alias.clone(),
                    graph: graph_alias.clone(),
                    payload: header,
                },
                &mut out,
            );
            for (key, node) in &graph.nodes {
                let mut payload = node.clone();
                let mut points = Vec::new();
                let mut choice_order = None;
                if node.type_id == "narrata.passage" {
                    match lower_builtin("passage", &node.data) {
                        Ok(SourcePlan::Passage(passage)) => {
                            points = passage.choice_points;
                            choice_order =
                                Some(points.iter().filter_map(|point| point.id).collect());
                            if let Some(data) = payload.data.as_object_mut() {
                                data.insert("choice_points".into(), serde_json::json!([]));
                            }
                        }
                        Err(error) => {
                            out.error(
                                Error::new(
                                    &error.code,
                                    format!(
                                        "packages.{package_alias}.graphs.{graph_alias}.nodes.{key}"
                                    ),
                                    error.message,
                                ),
                                None,
                            );
                            continue;
                        }
                        _ => {}
                    }
                }
                add(
                    DraftRecord::Node {
                        format_version: DraftVersion,
                        package: package_alias.clone(),
                        graph: graph_alias.clone(),
                        key: key.clone(),
                        choice_order,
                        payload,
                    },
                    &mut out,
                );
                if let Some(node) = node.id {
                    for payload in points {
                        add(
                            DraftRecord::ChoicePoint {
                                format_version: DraftVersion,
                                package: package_alias.clone(),
                                graph: graph_alias.clone(),
                                node,
                                payload,
                            },
                            &mut out,
                        );
                    }
                }
            }
        }
        for payload in &package.tombstones {
            add(
                DraftRecord::Tombstone {
                    format_version: DraftVersion,
                    package: package_alias.clone(),
                    payload: *payload,
                },
                &mut out,
            );
        }
    }
    out.finish(Some(records))
}

pub fn assemble_records(records: &[CheckedRecord]) -> Report<ProjectSource> {
    let mut out = Collector::new(100_000);
    let source = assemble_partial(records, &mut out);
    out.finish(source)
}

pub(crate) fn assemble_partial(
    records: &[CheckedRecord],
    out: &mut Collector,
) -> Option<ProjectSource> {
    let mut project = None;
    let mut packages = BTreeMap::<String, PackageSource>::new();
    let mut graphs = BTreeMap::<(String, String), GraphSource>::new();
    let mut nodes =
        BTreeMap::<(String, String, String), (NodeSource, Option<Vec<ChoicePointId>>)>::new();
    let mut points =
        BTreeMap::<ChoicePointId, (&DraftRecord, String, String, NodeId, ChoicePointSource)>::new();
    let mut tombstones = Vec::new();
    let mut keys = BTreeMap::<String, &DraftRecord>::new();
    let mut node_records = BTreeMap::new();
    let mut live = BTreeMap::<AuthoredId, &DraftRecord>::new();
    let mut deleted = BTreeMap::<AuthoredId, &DraftRecord>::new();
    fn duplicate(
        out: &mut Collector,
        record: &DraftRecord,
        first: &DraftRecord,
        pointer: &str,
        message: &str,
    ) {
        out.push(Diagnostic {
            severity: Severity::Error,
            code: "duplicate".into(),
            location: record.location(pointer),
            related: vec![first.location(pointer)],
            message: message.into(),
        });
    }
    for checked in records {
        if !out.step() {
            return None;
        }
        let record = checked.record();
        if let Some(first) = keys.insert(record.key(), record) {
            duplicate(out, record, first, "", "duplicate draft record key");
        }
        let mut ids = Vec::new();
        match record {
            DraftRecord::Project { payload, .. } => {
                if project.is_none() {
                    project = Some(payload.clone());
                }
            }
            DraftRecord::Package {
                package, payload, ..
            } => {
                if packages.insert(package.clone(), payload.clone()).is_some() {
                    out.error(
                        Error::new("duplicate", "/package", "duplicate package alias"),
                        Some(record),
                    );
                }
            }
            DraftRecord::Graph {
                package,
                graph,
                payload,
                ..
            } => {
                if graphs
                    .insert((package.clone(), graph.clone()), payload.clone())
                    .is_some()
                {
                    out.error(
                        Error::new("duplicate", "/graph", "duplicate graph alias"),
                        Some(record),
                    );
                }
            }
            DraftRecord::Node {
                package,
                graph,
                key,
                payload,
                choice_order,
                ..
            } => {
                if nodes
                    .insert(
                        (package.clone(), graph.clone(), key.clone()),
                        (payload.clone(), choice_order.clone()),
                    )
                    .is_some()
                {
                    out.error(
                        Error::new("duplicate", "/key", "duplicate node alias"),
                        Some(record),
                    );
                }
                ids.extend(payload.id.map(AuthoredId::Node));
                if let Some(id) = payload.id {
                    node_records.insert(id, record);
                }
            }
            DraftRecord::ChoicePoint {
                package,
                graph,
                node,
                payload,
                ..
            } => {
                if let Some(id) = payload.id {
                    points.insert(
                        id,
                        (
                            record,
                            package.clone(),
                            graph.clone(),
                            *node,
                            payload.clone(),
                        ),
                    );
                    ids.push(AuthoredId::ChoicePoint(id));
                }
                ids.extend(
                    payload
                        .options
                        .iter()
                        .filter_map(|option| option.id.map(AuthoredId::Option)),
                );
            }
            DraftRecord::Tombstone {
                package, payload, ..
            } => {
                if let Some(first) = deleted.insert(*payload, record) {
                    duplicate(out, record, first, "/payload", "duplicate tombstone ID");
                }
                tombstones.push((record, package.clone(), *payload));
            }
        }
        for id in ids {
            if !out.step() {
                return None;
            }
            if let Some(first) = live.insert(id, record) {
                duplicate(out, record, first, "/payload/id", "duplicate authored ID");
            }
        }
    }
    for (id, record) in &deleted {
        if !out.step() {
            return None;
        }
        if let Some(first) = live.get(id) {
            out.push(Diagnostic {
                severity: Severity::Error,
                code: "tombstone".into(),
                location: record.location("/payload"),
                related: vec![first.location("/payload/id")],
                message: format!("{id} is both live and deleted"),
            });
        }
    }
    let mut used = BTreeSet::new();
    for ((package, graph, key), (mut node, order)) in nodes {
        if !out.step() {
            return None;
        }
        if let Some(order) = order {
            let mut ordered = Vec::new();
            for (index, id) in order.iter().enumerate() {
                if !out.step() {
                    return None;
                }
                if let Some((record, owner_package, owner_graph, owner_node, point)) =
                    points.get(id)
                {
                    if owner_package != &package
                        || owner_graph != &graph
                        || Some(*owner_node) != node.id
                    {
                        out.error(
                            Error::new(
                                "owner",
                                "/node",
                                "choice point locator differs from its parent node",
                            ),
                            Some(record),
                        );
                    }
                    if !used.insert(*id) {
                        out.error(
                            Error::new(
                                "duplicate",
                                "/node",
                                "choice point is ordered by multiple nodes",
                            ),
                            Some(record),
                        );
                    }
                    ordered.push(point.clone());
                } else {
                    let parent = node.id.and_then(|id| node_records.get(&id)).copied();
                    out.error(
                        Error::new(
                            "reference",
                            format!("/choice_order/{index}"),
                            format!("missing choice point {id}"),
                        ),
                        parent,
                    );
                }
            }
            if let Some(data) = node.data.as_object_mut() {
                data.insert("choice_points".into(), serde_json::json!(ordered));
            }
        }
        if let Some(parent) = graphs.get_mut(&(package.clone(), graph.clone())) {
            parent.nodes.insert(key.clone(), node);
        } else {
            out.error(
                Error::new(
                    "reference",
                    format!("packages.{package}.graphs.{graph}.nodes.{key}"),
                    "node has no graph header",
                ),
                None,
            );
        }
    }
    for (id, (record, ..)) in &points {
        if !out.step() {
            return None;
        }
        if !used.contains(id) {
            out.error(
                Error::new(
                    "orphan",
                    "/node",
                    "choice point is not listed by its parent",
                ),
                Some(record),
            );
        }
    }
    for ((package, graph), source) in graphs {
        if !out.step() {
            return None;
        }
        if let Some(parent) = packages.get_mut(&package) {
            parent.graphs.insert(graph, source);
        } else {
            out.error(
                Error::new(
                    "reference",
                    format!("packages.{package}.graphs.{graph}"),
                    "graph has no package header",
                ),
                None,
            );
        }
    }
    for (record, package, id) in tombstones {
        if !out.step() {
            return None;
        }
        if let Some(parent) = packages.get_mut(&package) {
            parent.tombstones.push(id);
        } else {
            out.error(
                Error::new("reference", "/package", "tombstone has no package header"),
                Some(record),
            );
        }
    }
    let Some(manifest) = project else {
        out.error(Error::new("reference", "$", "missing project header"), None);
        out.push(Diagnostic {
            severity: Severity::Warning,
            code: "checks_skipped".into(),
            location: Location::default(),
            related: Vec::new(),
            message: "project checks require the missing project header".into(),
        });
        return None;
    };
    if manifest.packages.keys().ne(packages.keys()) {
        out.error(
            Error::new(
                "reference",
                "packages",
                "package headers differ from the manifest instances",
            ),
            None,
        );
    }
    let source = ProjectSource { manifest, packages };
    match normalize_source(&source) {
        Ok(source) => Some(source),
        Err(error) => {
            out.error(error, None);
            None
        }
    }
}

pub(crate) fn locate_diagnostics(records: &[CheckedRecord], diagnostics: &mut [Diagnostic]) {
    let mut scopes = BTreeMap::<String, (Location, bool)>::new();
    let points: BTreeMap<_, _> = records
        .iter()
        .filter_map(|checked| match checked.record() {
            DraftRecord::ChoicePoint { payload, .. } => payload.id.map(|id| (id, checked.record())),
            _ => None,
        })
        .collect();
    for checked in records {
        let record = checked.record();
        if let DraftRecord::Node {
            package,
            graph,
            key,
            choice_order: Some(order),
            payload,
            ..
        } = record
        {
            for (index, id) in order.iter().enumerate() {
                if let Some(point) = points.get(id) {
                    let prefix = format!(
                        "packages.{package}.graphs.{graph}.nodes.{key}.choice_points[{index}]"
                    );
                    let mut location = point.location("/payload");
                    location.node = payload.id;
                    scopes.insert(prefix.clone(), (location, false));
                    if let DraftRecord::ChoicePoint { payload, .. } = point {
                        for (index, option) in payload.options.iter().enumerate() {
                            let mut location = point.location(&format!("/payload/options/{index}"));
                            location.option = option.id;
                            scopes.insert(format!("{prefix}.options[{index}]"), (location, false));
                        }
                    }
                }
            }
        }
        let pointer = if matches!(
            record,
            DraftRecord::Project { .. }
                | DraftRecord::Package { .. }
                | DraftRecord::Graph { .. }
                | DraftRecord::Node { .. }
        ) {
            "/payload"
        } else {
            ""
        };
        let is_node = matches!(record, DraftRecord::Node { .. });
        scopes.insert(record.source_path(), (record.location(pointer), is_node));
        if let DraftRecord::Graph { package, graph, .. } = record {
            scopes.insert(
                format!("{package}.{graph}"),
                (record.location(pointer), false),
            );
        }
    }
    for diagnostic in diagnostics
        .iter_mut()
        .filter(|diagnostic| diagnostic.location.record.is_none())
    {
        let Some(path) = diagnostic.location.source_path.as_deref() else {
            continue;
        };
        if path.starts_with("records[") {
            continue;
        }
        let mut prefix = path;
        loop {
            if let Some((location, is_node)) = scopes.get(prefix) {
                let tail = path.strip_prefix(prefix).unwrap_or_default();
                let mut located = location.clone();
                located.source_path = Some(path.into());
                let pointer = source_pointer(tail);
                if *is_node
                    && !pointer.is_empty()
                    && !matches!(pointer.as_str(), "/id" | "/type_id")
                {
                    located.pointer.push_str("/data");
                }
                located.pointer.push_str(&pointer);
                diagnostic.location = located;
                break;
            }
            if prefix.is_empty() {
                break;
            }
            prefix = prefix
                .rfind(['.', '['])
                .map_or("", |index| &prefix[..index]);
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct OwnershipIndex {
    pub record: String,
    pub kind: String,
    pub package: Option<String>,
    pub graph: Option<String>,
    pub node: Option<NodeId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetStatus {
    Local,
    Passage,
    Control,
    Unresolved,
}

#[derive(Clone, Debug, Serialize)]
pub struct OptionIndex {
    pub record: String,
    pub package: String,
    pub graph: String,
    pub node: NodeId,
    pub choice_point: ChoicePointId,
    pub option: OptionId,
    pub ordinal: usize,
    pub source_unit: Option<ContentRef>,
    pub placement: Option<AnchorId>,
    pub reply: Option<narrata_kernel::content::Segment>,
    pub rejoin: Option<AnchorId>,
    pub outcome: String,
    pub target_node: Option<NodeId>,
    pub target_unit: Option<ContentRef>,
    pub target_status: TargetStatus,
    pub tags: Vec<ContentRef>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DraftIndexes {
    pub ownership: Vec<OwnershipIndex>,
    pub options: Vec<OptionIndex>,
}

pub fn derive_draft_indexes(records: &CheckedRecords) -> DraftIndexes {
    let mut ownership = Vec::new();
    let mut options = Vec::new();
    for checked in records.records() {
        let (kind, package, graph, node) = match checked.record() {
            DraftRecord::Project { .. } => ("project", None, None, None),
            DraftRecord::Package { package, .. } => ("package", Some(package.clone()), None, None),
            DraftRecord::Graph { package, graph, .. } => {
                ("graph", Some(package.clone()), Some(graph.clone()), None)
            }
            DraftRecord::Node {
                package,
                graph,
                payload,
                ..
            } => (
                "node",
                Some(package.clone()),
                Some(graph.clone()),
                payload.id,
            ),
            DraftRecord::Tombstone { package, .. } => {
                ("tombstone", Some(package.clone()), None, None)
            }
            DraftRecord::ChoicePoint {
                package,
                graph,
                node,
                payload,
                ..
            } => {
                let source_graph = records
                    .source()
                    .packages
                    .get(package)
                    .and_then(|package| package.graphs.get(graph));
                let parent = source_graph
                    .and_then(|graph| graph.nodes.values().find(|source| source.id == Some(*node)));
                let source_unit = parent
                    .and_then(|source| lower_builtin("passage", &source.data).ok())
                    .and_then(|plan| match plan {
                        SourcePlan::Passage(passage) => passage.body.map(|body| body.unit),
                        _ => None,
                    });
                if let Some(point) = payload.id {
                    for (ordinal, option) in payload.options.iter().enumerate() {
                        let Some(id) = option.id else {
                            continue;
                        };
                        let mut row = OptionIndex {
                            record: checked.key(),
                            package: package.clone(),
                            graph: graph.clone(),
                            node: *node,
                            choice_point: point,
                            option: id,
                            ordinal,
                            source_unit: source_unit.clone(),
                            placement: payload.placement.clone(),
                            reply: None,
                            rejoin: None,
                            outcome: String::new(),
                            target_node: None,
                            target_unit: None,
                            target_status: TargetStatus::Unresolved,
                            tags: option
                                .label
                                .iter()
                                .chain(option.reason.iter())
                                .cloned()
                                .collect(),
                        };
                        match &option.outcome {
                            OutcomeSource::Local { reply, rejoin } => {
                                row.outcome = "local".into();
                                row.target_status = TargetStatus::Local;
                                row.reply = reply.clone();
                                row.rejoin = rejoin.clone();
                            }
                            OutcomeSource::Branch { target } => {
                                row.outcome = "branch".into();
                                if let Some(target) =
                                    source_graph.and_then(|graph| graph.nodes.get(target))
                                {
                                    row.target_node = target.id;
                                    if target.type_id == "narrata.passage" {
                                        if let Ok(SourcePlan::Passage(passage)) =
                                            lower_builtin("passage", &target.data)
                                        {
                                            row.target_unit = passage.body.map(|body| body.unit);
                                            row.target_status = TargetStatus::Passage;
                                        }
                                    } else if builtin_kind(&target.type_id).is_some() {
                                        row.target_status = TargetStatus::Control;
                                    }
                                }
                            }
                        }
                        options.push(row);
                    }
                }
                (
                    "choice_point",
                    Some(package.clone()),
                    Some(graph.clone()),
                    Some(*node),
                )
            }
        };
        ownership.push(OwnershipIndex {
            record: checked.key(),
            kind: kind.into(),
            package,
            graph,
            node,
        });
    }
    ownership.sort_by(|a, b| a.record.cmp(&b.record));
    options.sort_by(|a, b| (&a.record, a.ordinal).cmp(&(&b.record, b.ordinal)));
    DraftIndexes { ownership, options }
}

/// The schema expresses storage shape; checked constructors additionally prove semantics.
pub fn schema() -> Schema {
    let mut value =
        serde_json::to_value(schema_for!(DraftRecord)).unwrap_or(serde_json::Value::Null);
    if let Some(definitions) = value
        .get_mut("$defs")
        .and_then(serde_json::Value::as_object_mut)
    {
        if let Some(project) = definitions.get_mut("ProjectManifest") {
            project["properties"]["format_version"] =
                serde_json::json!({"type":"integer","const":2});
        }
        if let Some(package) = definitions.get_mut("PackageSource") {
            package["properties"]["id"] =
                serde_json::json!({"type":"string","minLength":1,"maxLength":128});
            package["properties"]["version"] =
                serde_json::json!({"type":"string","minLength":1,"maxLength":64});
        }
        for name in ["NodeSource", "ChoicePointSource", "OptionSource"] {
            if let Some(definition) = definitions.get_mut(name) {
                if let Some(properties) = definition
                    .get_mut("properties")
                    .and_then(serde_json::Value::as_object_mut)
                    && let Some(id) = properties.get_mut("id")
                    && let Some(choices) = id.get("anyOf").and_then(serde_json::Value::as_array)
                    && let Some(non_null) = choices.iter().find(|choice| {
                        choice.get("type").and_then(serde_json::Value::as_str) != Some("null")
                    })
                {
                    *id = non_null.clone();
                }
                if let Some(required) = definition
                    .get_mut("required")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    required.push(serde_json::json!("id"));
                }
            }
        }
        if let Some(package) = definitions.get_mut("PackageSource") {
            package["properties"]["graphs"] =
                serde_json::json!({"type":"object","maxProperties":0});
            package["properties"]["tombstones"] = serde_json::json!({"type":"array","maxItems":0});
        }
        if let Some(graph) = definitions.get_mut("GraphSource") {
            graph["properties"]["nodes"] = serde_json::json!({"type":"object","maxProperties":0});
        }
    }
    if let Some(variants) = value
        .get_mut("oneOf")
        .and_then(serde_json::Value::as_array_mut)
    {
        for variant in variants {
            if variant["properties"]["kind"]["const"] == "node" {
                variant["allOf"] = serde_json::json!([
                    {"if":{"properties":{"payload":{"properties":{"type_id":{"const":"narrata.passage"}}}}},
                     "then":{"required":["choice_order"],"properties":{"choice_order":{"type":"array","items":{"$ref":"#/$defs/ChoicePointId"},"uniqueItems":true,"maxItems":64},"payload":{"properties":{"data":{"type":"object","required":["choice_points"],"properties":{"choice_points":{"type":"array","maxItems":0}}}}}}},
                     "else":{"not":{"required":["choice_order"]}}}
                ]);
            }
        }
    }
    Schema::try_from(value).unwrap_or_else(|_| json_schema!({"not":{}}))
}

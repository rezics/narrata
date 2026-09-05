use std::{collections::BTreeMap, sync::Arc};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Assignment, CheckedProduct, Error, FORMAT_VERSION, GraphRef, NodeAddress, NodePlan, Result,
    Scalar, ScalarType, Scope,
    compile::{canonical_bytes, digest},
    expr::{self, Values},
    parse_json,
};

const MAX_STEPS: u32 = 4096;
const MAX_DEPTH: usize = 64;
const MAX_COMMITS: usize = 512;
const MAX_REPLAY_STEPS: u64 = 1_000_000;
const MAX_STATE_BYTES: usize = 128 * 1024;
const MAX_RETAINED_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
struct Frame {
    graph: GraphRef,
    node: String,
    instance: u32,
    parameters: BTreeMap<String, Scalar>,
    locals: BTreeMap<String, Scalar>,
}

#[derive(Clone, Debug, Serialize)]
struct State {
    shared: BTreeMap<String, Scalar>,
    frames: Vec<Frame>,
    next_instance: u32,
    finished: Option<(NodeAddress, u32, String)>,
}

impl Frame {
    fn address(&self) -> NodeAddress {
        NodeAddress {
            package: self.graph.package.clone(),
            graph: self.graph.graph.clone(),
            node: self.node.clone(),
        }
    }
}

impl State {
    fn frame(&self) -> Result<&Frame> {
        self.frames
            .last()
            .ok_or_else(|| Error::new("finished", "session", "no active graph instance"))
    }
    fn frame_mut(&mut self) -> Result<&mut Frame> {
        self.frames
            .last_mut()
            .ok_or_else(|| Error::new("finished", "session", "no active graph instance"))
    }
    fn values(&self) -> Result<Values<'_>> {
        let frame = self.frame()?;
        Ok(Values {
            parameters: &frame.parameters,
            locals: &frame.locals,
            shared: &self.shared,
        })
    }
    fn assign(&mut self, assignments: &[Assignment]) -> Result<()> {
        for assignment in assignments {
            let value = self.values()?.evaluate(&assignment.value)?;
            let target = match assignment.target.scope {
                Scope::Local => self.frame_mut()?.locals.get_mut(&assignment.target.name),
                Scope::Shared => self.shared.get_mut(&assignment.target.name),
                Scope::Parameter => {
                    return Err(Error::new(
                        "readonly",
                        "assignment",
                        "parameters cannot be modified",
                    ));
                }
            }
            .ok_or_else(|| {
                Error::new("state", "assignment", "missing checked assignment target")
            })?;
            expr::expect(value.kind(), target.kind(), "assignment")?;
            *target = value;
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Commit {
    id: String,
    parent: Option<String>,
    action: Option<String>,
    state: Arc<State>,
    title: String,
    node: NodeAddress,
    instance: u32,
}

/// Single-process reference coordinator. Host persistence must finish before publishing a new view.
/// No I/O or external effects occur in this Gamebook profile.
#[derive(Clone)]
pub struct Session {
    product: Arc<CheckedProduct>,
    commits: Vec<Commit>,
    index: BTreeMap<String, usize>,
    cursor: usize,
    last_steps: u32,
    retained_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionView {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub reason: Option<String>,
}

/// Scalar payloads are formatted strings, so 64-bit integers never lose precision in JavaScript.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VariableView {
    pub name: String,
    pub kind: ScalarType,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrameView {
    pub graph: GraphRef,
    pub node: String,
    pub instance: u32,
    pub parameters: Vec<VariableView>,
    pub locals: Vec<VariableView>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryView {
    pub id: String,
    pub parent: Option<String>,
    pub title: String,
    pub node: NodeAddress,
    pub instance: u32,
    pub current: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionView {
    pub artifact_id: String,
    pub cursor: String,
    pub product_title: String,
    pub node: NodeAddress,
    pub instance: u32,
    pub title: String,
    pub paragraphs: Vec<String>,
    pub actions: Vec<ActionView>,
    pub finished: bool,
    pub outcome: Option<String>,
    pub shared: Vec<VariableView>,
    pub frames: Vec<FrameView>,
    pub history: Vec<HistoryView>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedCommit {
    pub id: String,
    pub parent: Option<String>,
    pub action: Option<String>,
    /// Full logical state. Restore validates it against both its digest and deterministic replay.
    pub snapshot: serde_json::Value,
}

/// Bounded, fully replay-validated save for the R1 reference profile. It preserves every retained
/// branch. It is not a legacy CheckpointBundle or an authentication/signature format.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SaveArchive {
    pub format_version: u16,
    pub artifact_id: String,
    pub commits: Vec<SavedCommit>,
    pub cursor: String,
}

fn variables(map: &BTreeMap<String, Scalar>) -> Vec<VariableView> {
    map.iter()
        .map(|(name, value)| VariableView {
            name: name.clone(),
            kind: value.kind(),
            value: value.display(),
        })
        .collect()
}

impl Session {
    pub fn new(product: Arc<CheckedProduct>) -> Result<Self> {
        let graph = product.graph(&product.source.product.entry)?;
        let mut state = State {
            shared: product.source.product.shared.clone(),
            frames: vec![Frame {
                graph: product.source.product.entry.clone(),
                node: graph.source.entry.clone(),
                instance: 1,
                parameters: product.source.product.arguments.clone(),
                locals: graph.source.locals.clone(),
            }],
            next_instance: 2,
            finished: None,
        };
        let last_steps = settle(&product, &mut state)?;
        let retained_bytes = check_state_size(&state)?;
        let preview = render(&product, &state)?;
        let id = digest(
            "NARRATA-NODES-COMMIT-1",
            &(
                product.artifact_id(),
                Option::<&str>::None,
                Option::<&str>::None,
                &state,
            ),
        )?;
        let root = Commit {
            id: id.clone(),
            parent: None,
            action: None,
            state: Arc::new(state),
            title: preview.title,
            node: preview.node,
            instance: preview.instance,
        };
        Ok(Self {
            product,
            commits: vec![root],
            index: BTreeMap::from([(id, 0)]),
            cursor: 0,
            last_steps,
            retained_bytes,
        })
    }

    fn commit(&self) -> Result<&Commit> {
        self.commits
            .get(self.cursor)
            .ok_or_else(|| Error::new("state", "cursor", "missing commit"))
    }
    pub fn cursor(&self) -> Result<&str> {
        Ok(&self.commit()?.id)
    }
    pub fn product(&self) -> &CheckedProduct {
        &self.product
    }

    pub fn view(&self) -> Result<SessionView> {
        let commit = self.commit()?;
        let preview = render(&self.product, &commit.state)?;
        Ok(SessionView {
            artifact_id: self.product.artifact_id().into(),
            cursor: commit.id.clone(),
            product_title: self.product.title().into(),
            node: preview.node,
            instance: preview.instance,
            title: preview.title,
            paragraphs: preview.paragraphs,
            actions: preview.actions,
            finished: preview.outcome.is_some(),
            outcome: preview.outcome,
            shared: variables(&commit.state.shared),
            frames: commit
                .state
                .frames
                .iter()
                .map(|f| FrameView {
                    graph: f.graph.clone(),
                    node: f.node.clone(),
                    instance: f.instance,
                    parameters: variables(&f.parameters),
                    locals: variables(&f.locals),
                })
                .collect(),
            history: self
                .commits
                .iter()
                .enumerate()
                .map(|(i, c)| HistoryView {
                    id: c.id.clone(),
                    parent: c.parent.clone(),
                    title: c.title.clone(),
                    node: c.node.clone(),
                    instance: c.instance,
                    current: self.cursor == i,
                })
                .collect(),
        })
    }

    pub fn select(&mut self, expected_commit: &str, action: &str) -> Result<SessionView> {
        if self.cursor()? != expected_commit {
            return Err(Error::new(
                "stale_input",
                "expected_commit",
                "the displayed interaction is no longer current",
            ));
        }
        let parent = self.commit()?.clone();
        let mut candidate = parent.state.as_ref().clone();
        let frame = candidate.frame()?;
        let plan = self
            .product
            .graph(&frame.graph)?
            .nodes
            .get(&frame.node)
            .ok_or_else(|| Error::new("state", &frame.node, "missing current node"))?
            .clone();
        match plan {
            NodePlan::Content { next, .. } if action == "continue" => {
                candidate.frame_mut()?.node = next
            }
            NodePlan::Decision { choices, .. } => {
                let choice = choices
                    .iter()
                    .find(|c| c.id == action)
                    .ok_or_else(|| Error::new("action", "action", "unknown choice"))?;
                let values = candidate.values()?;
                if !condition(&values, choice.visible_if.as_ref())?
                    || !condition(&values, choice.enabled_if.as_ref())?
                {
                    return Err(Error::new(
                        "unavailable",
                        "action",
                        "choice conditions are not satisfied",
                    ));
                }
                candidate.assign(&choice.assignments)?;
                candidate.frame_mut()?.node = choice.target.clone();
            }
            _ => {
                return Err(Error::new(
                    "action",
                    "action",
                    "action does not belong to the current interaction",
                ));
            }
        }
        let steps = settle(&self.product, &mut candidate)?;
        let preview = render(&self.product, &candidate)?;
        let state_bytes = check_state_size(&candidate)?;
        let id = digest(
            "NARRATA-NODES-COMMIT-1",
            &(
                self.product.artifact_id(),
                Some(&parent.id),
                Some(action),
                &candidate,
            ),
        )?;
        if let Some(index) = self.index.get(&id).copied() {
            self.cursor = index;
        } else {
            if self.commits.len() >= MAX_COMMITS
                || self.retained_bytes + state_bytes > MAX_RETAINED_BYTES
            {
                return Err(Error::new(
                    "history_limit",
                    "session",
                    "R1 retention limit reached (512 commits / 2 MiB of snapshots); export the journey and start a new one",
                ));
            }
            let index = self.commits.len();
            self.commits.push(Commit {
                id: id.clone(),
                parent: Some(parent.id),
                action: Some(action.into()),
                state: Arc::new(candidate),
                title: preview.title,
                node: preview.node,
                instance: preview.instance,
            });
            self.index.insert(id, index);
            self.cursor = index;
            self.retained_bytes += state_bytes;
        }
        self.last_steps = steps;
        self.view()
    }

    pub fn checkout(&mut self, commit: &str) -> Result<SessionView> {
        let index = self.index.get(commit).copied().ok_or_else(|| {
            Error::new(
                "reference",
                "commit",
                "commit is not retained in this session",
            )
        })?;
        // Rendering is checked before moving the visible cursor.
        let target = self
            .commits
            .get(index)
            .ok_or_else(|| Error::new("state", "commit", "missing indexed commit"))?;
        render(&self.product, &target.state)?;
        self.cursor = index;
        self.view()
    }

    pub fn save(&self) -> Result<String> {
        let archive = SaveArchive {
            format_version: FORMAT_VERSION,
            artifact_id: self.product.artifact_id().into(),
            cursor: self.cursor()?.into(),
            commits: self
                .commits
                .iter()
                .map(|c| {
                    Ok(SavedCommit {
                        id: c.id.clone(),
                        parent: c.parent.clone(),
                        action: c.action.clone(),
                        snapshot: serde_json::to_value(c.state.as_ref())
                            .map_err(|e| Error::new("encoding", "snapshot", e.to_string()))?,
                    })
                })
                .collect::<Result<_>>()?,
        };
        String::from_utf8(canonical_bytes(&archive)?)
            .map_err(|e| Error::new("encoding", "save", e.to_string()))
    }

    pub fn restore(product: Arc<CheckedProduct>, text: &str) -> Result<Self> {
        let archive: SaveArchive = parse_json(text)?;
        if archive.format_version != FORMAT_VERSION || archive.artifact_id != product.artifact_id()
        {
            return Err(Error::new(
                "incompatible_save",
                "artifact_id",
                "save requires its exact node product and format",
            ));
        }
        if archive.commits.is_empty() || archive.commits.len() > MAX_COMMITS {
            return Err(Error::new(
                "limit",
                "commits",
                "invalid retained history length",
            ));
        }
        let mut session = Self::new(product)?;
        let first = archive
            .commits
            .first()
            .ok_or_else(|| Error::new("save", "commits", "missing root"))?;
        if first.id != session.cursor()? || first.parent.is_some() || first.action.is_some() {
            return Err(Error::new(
                "save",
                "commits[0]",
                "root does not match the initial state",
            ));
        }
        check_snapshot(
            &first.snapshot,
            &session.commit()?.state,
            "commits[0].snapshot",
        )?;
        let mut work = u64::from(session.last_steps);
        for (i, commit) in archive.commits.iter().enumerate().skip(1) {
            if session.index.contains_key(&commit.id) {
                return Err(Error::new(
                    "duplicate",
                    format!("commits[{i}]"),
                    "duplicate commit",
                ));
            }
            let parent = commit.parent.as_deref().ok_or_else(|| {
                Error::new(
                    "save",
                    format!("commits[{i}]"),
                    "non-root commit needs a parent",
                )
            })?;
            let action = commit.action.as_deref().ok_or_else(|| {
                Error::new(
                    "save",
                    format!("commits[{i}]"),
                    "non-root commit needs an action",
                )
            })?;
            session.checkout(parent)?;
            session.select(parent, action)?;
            if session.cursor()? != commit.id {
                return Err(Error::new(
                    "save",
                    format!("commits[{i}]"),
                    "replayed state does not match the commit digest",
                ));
            }
            check_snapshot(
                &commit.snapshot,
                &session.commit()?.state,
                &format!("commits[{i}].snapshot"),
            )?;
            work += u64::from(session.last_steps);
            if work > MAX_REPLAY_STEPS {
                return Err(Error::new(
                    "limit",
                    "save",
                    "save replay exceeds the work budget",
                ));
            }
        }
        session.checkout(&archive.cursor)?;
        Ok(session)
    }
}

fn settle(product: &CheckedProduct, state: &mut State) -> Result<u32> {
    for steps in 1..=MAX_STEPS {
        let frame = state.frame()?.clone();
        let graph = product.graph(&frame.graph)?;
        let plan = graph
            .nodes
            .get(&frame.node)
            .ok_or_else(|| Error::new("state", &frame.node, "unknown checked node"))?;
        match plan {
            NodePlan::Content { .. } | NodePlan::Decision { .. } => return Ok(steps),
            NodePlan::Branch {
                condition,
                when_true,
                when_false,
            } => {
                let next = if state.values()?.boolean(condition)? {
                    when_true
                } else {
                    when_false
                };
                state.frame_mut()?.node = next.clone();
            }
            NodePlan::Mutate { assignments, next } => {
                state.assign(assignments)?;
                state.frame_mut()?.node = next.clone();
                check_state_size(state)?;
            }
            NodePlan::Call {
                target, arguments, ..
            } => {
                if state.frames.len() >= MAX_DEPTH {
                    return Err(Error::new(
                        "depth_limit",
                        frame.graph.label(),
                        "subgraph call depth exceeds 64",
                    ));
                }
                let target = product.target(&frame.graph, target)?;
                let callee = product.graph(&target)?;
                let parameters = arguments
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), state.values()?.evaluate(value)?)))
                    .collect::<Result<_>>()?;
                let instance = state.next_instance;
                state.next_instance = instance
                    .checked_add(1)
                    .ok_or_else(|| Error::new("limit", "instances", "instance counter overflow"))?;
                state.frames.push(Frame {
                    graph: target,
                    node: callee.source.entry.clone(),
                    instance,
                    parameters,
                    locals: callee.source.locals.clone(),
                });
                check_state_size(state)?;
            }
            NodePlan::Return { outcome } => {
                state.frames.pop();
                if let Some(parent) = state.frames.last_mut() {
                    let plan = product
                        .graph(&parent.graph)?
                        .nodes
                        .get(&parent.node)
                        .ok_or_else(|| Error::new("state", &parent.node, "missing call site"))?;
                    let NodePlan::Call { on_return, .. } = plan else {
                        return Err(Error::new("state", "return", "parent is not a call site"));
                    };
                    parent.node = on_return.get(outcome).cloned().ok_or_else(|| {
                        Error::new("state", "return", "missing outcome continuation")
                    })?;
                } else {
                    state.finished = Some((frame.address(), frame.instance, outcome.clone()));
                    return Ok(steps);
                }
            }
        }
    }
    Err(Error::new(
        "step_limit",
        "execution",
        "automatic node transitions exceeded 4096 steps without an interaction",
    ))
}

fn check_state_size(state: &State) -> Result<usize> {
    let size = canonical_bytes(state)?.len();
    if size > MAX_STATE_BYTES {
        return Err(Error::new(
            "limit",
            "state",
            "R1 logical state exceeds 128 KiB",
        ));
    }
    Ok(size)
}

fn check_snapshot(snapshot: &serde_json::Value, state: &State, path: &str) -> Result<()> {
    if canonical_bytes(snapshot)? != canonical_bytes(state)? {
        return Err(Error::new(
            "save",
            path,
            "snapshot does not match the checked replayed state",
        ));
    }
    Ok(())
}

struct Preview {
    node: NodeAddress,
    instance: u32,
    title: String,
    paragraphs: Vec<String>,
    actions: Vec<ActionView>,
    outcome: Option<String>,
}

fn condition(values: &Values<'_>, expr: Option<&crate::Expr>) -> Result<bool> {
    expr.map(|v| values.boolean(v)).unwrap_or(Ok(true))
}

fn render(product: &CheckedProduct, state: &State) -> Result<Preview> {
    if let Some((node, instance, outcome)) = &state.finished {
        return Ok(Preview {
            node: node.clone(),
            instance: *instance,
            title: "旅程结束".into(),
            paragraphs: vec!["这段旅程已经结束。你可以回到任一历史节点，尝试另一条路线。".into()],
            actions: Vec::new(),
            outcome: Some(outcome.clone()),
        });
    }
    let frame = state.frame()?;
    let plan = product
        .graph(&frame.graph)?
        .nodes
        .get(&frame.node)
        .ok_or_else(|| Error::new("state", &frame.node, "unknown node"))?;
    let values = state.values()?;
    let (content, actions) = match plan {
        NodePlan::Content { content, label, .. } => (
            content,
            vec![ActionView {
                id: "continue".into(),
                label: values.render(label)?,
                enabled: true,
                reason: None,
            }],
        ),
        NodePlan::Decision { content, choices } => {
            let mut actions = Vec::new();
            for choice in choices {
                if !condition(&values, choice.visible_if.as_ref())? {
                    continue;
                }
                let enabled = condition(&values, choice.enabled_if.as_ref())?;
                actions.push(ActionView {
                    id: choice.id.clone(),
                    label: values.render(&choice.label)?,
                    enabled,
                    reason: if enabled {
                        None
                    } else {
                        Some(
                            choice
                                .disabled_reason
                                .as_deref()
                                .map(|v| values.render(v))
                                .transpose()?
                                .unwrap_or_else(|| "条件尚未满足".into()),
                        )
                    },
                });
            }
            if !actions.iter().any(|a| a.enabled) {
                return Err(Error::new(
                    "no_actions",
                    frame.graph.label(),
                    "decision has no available action",
                ));
            }
            (content, actions)
        }
        _ => {
            return Err(Error::new(
                "state",
                &frame.node,
                "session is not at an interaction boundary",
            ));
        }
    };
    let body = product
        .source
        .packages
        .get(&frame.graph.package)
        .and_then(|p| p.content.get(content))
        .ok_or_else(|| Error::new("state", content, "missing compiled content"))?;
    Ok(Preview {
        node: frame.address(),
        instance: frame.instance,
        title: values.render(&body.title)?,
        paragraphs: body
            .paragraphs
            .iter()
            .map(|p| values.render(p))
            .collect::<Result<_>>()?,
        actions,
        outcome: None,
    })
}

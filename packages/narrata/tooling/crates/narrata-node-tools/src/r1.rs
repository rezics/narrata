//! Reading R1 projects (ADR 0011) to migrate them (ADR 0013 §10). R1 never runs; its
//! sources are converted to R2 sources plus a local content pack that holds every text,
//! including R1's built-in defaults and ending page.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use narrata_content_local::{Block, BlocksEntry, ContentPack, Entry, TextEntry};
use narrata_kernel::content::{AnchorId, ContentKey, ContentRef, ProviderId, Segment};
use narrata_nodes::{
    Assignment, Error, Expr, PackageSource, ProjectManifest, Result, Scalar, ScalarType, Scope,
    canonical_json,
    plan::{CallTarget, GraphRef, Signature},
    source::{
        Binding, ChoicePointSource, EndingSource, GraphSource, NodeSource, OptionSource,
        OutcomeSource, PassageSource, ProductSource, R1ArtifactId,
    },
};
use serde::{Deserialize, Serialize};

use crate::{
    files::{package_path, read_text},
    ids::Minter,
};

/// R1's built-in texts, which R2 keeps in the content pack.
const DEFAULT_CONTINUE: &str = "继续";
const DEFAULT_DISABLED_REASON: &str = "条件尚未满足";
const ENDING_TITLE: &str = "旅程结束";
const ENDING_BODY: &str = "这段旅程已经结束。你可以回到任一历史节点，尝试另一条路线。";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub format_version: u16,
    pub product: Product,
    pub packages: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
struct Bundle<'a> {
    format_version: u16,
    product: &'a Product,
    packages: &'a BTreeMap<String, Package>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Product {
    pub id: String,
    pub title: String,
    pub entry: GraphRef,
    #[serde(default)]
    pub arguments: BTreeMap<String, Scalar>,
    #[serde(default)]
    pub shared: BTreeMap<String, Scalar>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub shared_labels: BTreeMap<String, String>,
    #[serde(default)]
    pub bindings: Vec<Binding>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub id: String,
    pub version: String,
    pub exports: Vec<String>,
    #[serde(default)]
    pub content: BTreeMap<String, Content>,
    pub graphs: BTreeMap<String, Graph>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    pub title: String,
    pub paragraphs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    pub title: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, ScalarType>,
    #[serde(default)]
    pub locals: BTreeMap<String, Scalar>,
    #[serde(default)]
    pub shared: BTreeMap<String, ScalarType>,
    #[serde(default)]
    pub imports: BTreeMap<String, Signature>,
    pub outcomes: Vec<String>,
    pub entry: String,
    pub nodes: BTreeMap<String, Node>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub type_id: String,
    pub data: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Plan {
    Content {
        content: String,
        label: String,
        next: String,
    },
    Decision {
        content: String,
        choices: Vec<Choice>,
    },
    Branch {
        condition: Expr,
        when_true: String,
        when_false: String,
    },
    Mutate {
        assignments: Vec<Assignment>,
        next: String,
    },
    Call {
        target: CallTarget,
        arguments: BTreeMap<String, Expr>,
        on_return: BTreeMap<String, String>,
    },
    Return {
        outcome: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Choice {
    id: String,
    label: String,
    #[serde(default)]
    visible_if: Option<Expr>,
    #[serde(default)]
    enabled_if: Option<Expr>,
    #[serde(default)]
    disabled_reason: Option<String>,
    #[serde(default)]
    assignments: Vec<Assignment>,
    target: String,
}

/// R1 lowering: the kind comes from `type_id`, with R1's defaults filled in.
fn lower(node: &Node, path: &str) -> Result<Plan> {
    let kind = node
        .type_id
        .strip_prefix("narrata.")
        .filter(|kind| ["content", "decision", "branch", "mutate", "call", "return"].contains(kind))
        .ok_or_else(|| {
            Error::new(
                "node_type",
                path,
                format!("{} is not an R1 node type", node.type_id),
            )
        })?;
    let mut fields = node
        .data
        .as_object()
        .cloned()
        .ok_or_else(|| Error::new("schema", path, "node data must be an object"))?;
    fields.insert("kind".into(), serde_json::json!(kind));
    let default = match kind {
        "content" => Some(("label", serde_json::json!(DEFAULT_CONTINUE))),
        "mutate" => Some(("assignments", serde_json::json!([]))),
        "call" => Some(("arguments", serde_json::json!({}))),
        _ => None,
    };
    if let Some((field, value)) = default {
        fields.entry(field).or_insert(value);
    }
    serde_json::from_value(serde_json::Value::Object(fields))
        .map_err(|e| Error::new("schema", path, e.to_string()))
}

/// An R1 project with its packages, as R1 normalized them for its artifact ID.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub project: Project,
    pub packages: BTreeMap<String, Package>,
}

pub fn load(path: &Path) -> Result<Loaded> {
    let manifest_path = path
        .canonicalize()
        .map_err(|e| crate::files::io_error(path, e))?;
    let root = manifest_path.parent().ok_or_else(|| {
        Error::new(
            "path",
            path.display().to_string(),
            "manifest needs a parent directory",
        )
    })?;
    let mut project: Project = narrata_nodes::parse_json(&read_text(&manifest_path)?)?;
    if project.format_version != 1 {
        return Err(Error::new("version", "format_version", "not an R1 project"));
    }
    let mut packages = BTreeMap::new();
    for (alias, relative) in &project.packages {
        let mut package: Package =
            narrata_nodes::parse_json(&read_text(&package_path(root, alias, relative)?)?)?;
        package.exports.sort();
        for graph in package.graphs.values_mut() {
            graph.outcomes.sort();
            for signature in graph.imports.values_mut() {
                signature.outcomes.sort();
            }
        }
        packages.insert(alias.clone(), package);
    }
    project.product.bindings.sort_by(|a, b| a.from.cmp(&b.from));
    Ok(Loaded { project, packages })
}

#[derive(Serialize)]
struct CompiledGraph<'a> {
    source: &'a Graph,
    nodes: BTreeMap<String, Plan>,
}

impl Loaded {
    /// The R1 `artifact_id`: SHA-256 over the domain, a zero byte and the canonical JSON of
    /// the normalized bundle, the node type revisions and every graph with its lowered plans.
    pub fn artifact_id(&self) -> Result<R1ArtifactId> {
        let mut node_types = BTreeMap::new();
        let mut graphs = Vec::new();
        for (alias, package) in &self.packages {
            for (name, graph) in &package.graphs {
                let mut nodes = BTreeMap::new();
                for (id, node) in &graph.nodes {
                    nodes.insert(id.clone(), lower(node, &format!("{alias}.{name}.{id}"))?);
                    node_types.insert(node.type_id.clone(), "1".to_owned());
                }
                graphs.push((
                    GraphRef {
                        package: alias.clone(),
                        graph: name.clone(),
                    },
                    CompiledGraph {
                        source: graph,
                        nodes,
                    },
                ));
            }
        }
        let bundle = Bundle {
            format_version: 1,
            product: &self.project.product,
            packages: &self.packages,
        };
        let mut bytes = b"NARRATA-NODES-PRODUCT-1\0".to_vec();
        bytes.extend(canonical_json(&(&bundle, &node_types, &graphs))?);
        Ok(R1ArtifactId(narrata_kernel::codec::sha256(&bytes)))
    }
}

/// The R2 sources and content pack a migration writes.
#[derive(Clone, Debug)]
pub struct Migrated {
    pub manifest: ProjectManifest,
    pub packages: BTreeMap<String, PackageSource>,
    pub content: ContentPack,
}

struct Texts {
    entries: BTreeMap<ContentKey, Entry>,
}

impl Texts {
    fn key(text: &str) -> Result<ContentKey> {
        ContentKey::new(text).map_err(|e| Error::new("content", text, e.to_string()))
    }

    fn reference(key: &str) -> Result<ContentRef> {
        ContentRef::new("local", key).map_err(|e| Error::new("content", key, e.to_string()))
    }

    fn text(&mut self, key: &str, text: String) -> Result<ContentRef> {
        self.entries
            .insert(Self::key(key)?, Entry::Text(TextEntry { text }));
        Self::reference(key)
    }

    fn blocks(&mut self, key: &str, paragraphs: Vec<String>) -> Result<Segment> {
        let blocks = paragraphs
            .into_iter()
            .enumerate()
            .map(|(index, text)| {
                Ok(Block::Text {
                    id: anchor(index)?,
                    text,
                })
            })
            .collect::<Result<_>>()?;
        self.entries
            .insert(Self::key(key)?, Entry::Blocks(BlocksEntry { blocks }));
        Ok(Segment {
            unit: Self::reference(key)?,
            first: None,
            last: None,
        })
    }
}

fn anchor(index: usize) -> Result<AnchorId> {
    AnchorId::new(format!("p{}", index + 1))
        .map_err(|e| Error::new("content", "anchor", e.to_string()))
}

/// The `{{scope.name}}` placeholders of an R1 text, in order.
fn placeholders(text: &str, path: &str) -> Result<Vec<(usize, usize, Scope, String)>> {
    let mut found = Vec::new();
    let mut offset = 0;
    while let Some(start) = text[offset..].find("{{") {
        let start = offset + start;
        let end = text[start + 2..]
            .find("}}")
            .map(|index| start + 2 + index + 2)
            .ok_or_else(|| Error::new("template", path, "unclosed {{variable}}"))?;
        let field = text[start + 2..end - 2].trim();
        let (scope, name) = field
            .split_once('.')
            .ok_or_else(|| Error::new("template", path, "use {{scope.name}}"))?;
        let scope = match scope {
            "parameter" => Scope::Parameter,
            "local" => Scope::Local,
            "shared" => Scope::Shared,
            _ => {
                return Err(Error::new(
                    "template",
                    path,
                    format!("unknown scope {scope}"),
                ));
            }
        };
        found.push((start, end, scope, name.to_owned()));
        offset = end;
    }
    Ok(found)
}

fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Parameter => "parameter",
        Scope::Local => "local",
        Scope::Shared => "shared",
    }
}

/// Names placeholders after their variable; names used in two scopes within a package get a
/// scope prefix everywhere in that package.
struct Naming {
    conflicts: BTreeSet<String>,
}

impl Naming {
    fn argument(&self, scope: Scope, name: &str) -> String {
        if self.conflicts.contains(name) {
            format!("{}_{name}", scope_name(scope))
        } else {
            name.to_owned()
        }
    }

    /// Rewrites a text to `{name}` placeholders with literal braces escaped, and records the
    /// variables it reads.
    fn rewrite(
        &self,
        text: &str,
        path: &str,
        reads: &mut BTreeMap<String, (Scope, String)>,
    ) -> Result<String> {
        let escape = |part: &str| part.replace('{', "{{").replace('}', "}}");
        let mut output = String::new();
        let mut offset = 0;
        for (start, end, scope, name) in placeholders(text, path)? {
            output.push_str(&escape(&text[offset..start]));
            let argument = self.argument(scope, &name);
            output.push('{');
            output.push_str(&argument);
            output.push('}');
            reads.insert(argument, (scope, name));
            offset = end;
        }
        output.push_str(&escape(&text[offset..]));
        Ok(output)
    }
}

fn package_naming(package: &Package) -> Result<Naming> {
    let mut scopes = BTreeMap::<String, BTreeSet<&'static str>>::new();
    let mut note = |text: &str| -> Result<()> {
        for (_, _, scope, name) in placeholders(text, "content")? {
            scopes.entry(name).or_default().insert(scope_name(scope));
        }
        Ok(())
    };
    for content in package.content.values() {
        note(&content.title)?;
        for paragraph in &content.paragraphs {
            note(paragraph)?;
        }
    }
    for graph in package.graphs.values() {
        for node in graph.nodes.values() {
            for text in ["label", "disabled_reason"] {
                for value in node.data.get(text).into_iter().chain(
                    node.data
                        .get("choices")
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|choice| choice.get(text)),
                ) {
                    if let Some(value) = value.as_str() {
                        note(value)?;
                    }
                }
            }
        }
    }
    Ok(Naming {
        conflicts: scopes
            .into_iter()
            .filter(|(_, scopes)| scopes.len() > 1)
            .map(|(name, _)| name)
            .collect(),
    })
}

impl Loaded {
    /// Converts the project to R2 sources with freshly minted IDs and a content pack in
    /// `language`. Node aliases are the R1 node IDs; option keys are the R1 choice IDs, and a
    /// content node's single option is `continue`, so R1 actions map to option keys.
    pub fn migrate(&self, language: &str, minter: &mut Minter) -> Result<Migrated> {
        let mut texts = Texts {
            entries: BTreeMap::new(),
        };
        let product = &self.project.product;
        let entry_graph = self
            .packages
            .get(&product.entry.package)
            .and_then(|package| package.graphs.get(&product.entry.graph))
            .ok_or_else(|| {
                Error::new("reference", "product.entry", "entry graph does not exist")
            })?;
        let ending_title = texts.text("product:ending.title", ENDING_TITLE.into())?;
        let ending_body = texts.blocks("product:ending", vec![ENDING_BODY.into()])?;
        let manifest = ProjectManifest {
            format_version: narrata_nodes::FORMAT_VERSION,
            product: ProductSource {
                id: product.id.clone(),
                title: Some(texts.text("product:title", product.title.clone())?),
                entry: product.entry.clone(),
                arguments: product.arguments.clone(),
                shared: product.shared.clone(),
                shared_labels: product
                    .shared_labels
                    .iter()
                    .map(|(name, label)| {
                        Ok((
                            name.clone(),
                            texts.text(&format!("product:shared.{name}"), label.clone())?,
                        ))
                    })
                    .collect::<Result<_>>()?,
                endings: entry_graph
                    .outcomes
                    .iter()
                    .map(|outcome| {
                        (
                            outcome.clone(),
                            EndingSource {
                                title: Some(ending_title.clone()),
                                body: Some(ending_body.clone()),
                            },
                        )
                    })
                    .collect(),
                bindings: product.bindings.clone(),
            },
            packages: self.project.packages.clone(),
            migrated_from_r1: Some(self.artifact_id()?),
        };
        let mut packages = BTreeMap::new();
        for (alias, package) in &self.packages {
            packages.insert(
                alias.clone(),
                self.migrate_package(alias, package, &mut texts, minter)?,
            );
        }
        Ok(Migrated {
            manifest,
            packages,
            content: ContentPack {
                format_version: narrata_content_local::FORMAT_VERSION,
                provider: ProviderId::new("local")
                    .map_err(|e| Error::new("content", "provider", e.to_string()))?,
                language: language.to_owned(),
                entries: texts.entries,
            },
        })
    }

    fn migrate_package(
        &self,
        alias: &str,
        package: &Package,
        texts: &mut Texts,
        minter: &mut Minter,
    ) -> Result<PackageSource> {
        let naming = package_naming(package)?;
        let mut graphs = BTreeMap::new();
        for (name, graph) in &package.graphs {
            let mut nodes = BTreeMap::new();
            for (id, node) in &graph.nodes {
                let path = format!("packages.{alias}.graphs.{name}.nodes.{id}");
                let (type_id, data) = match lower(node, &path)? {
                    Plan::Content {
                        content,
                        label,
                        next,
                    } => {
                        let label = if label == DEFAULT_CONTINUE {
                            Rewritten::default_text("default:continue", DEFAULT_CONTINUE)
                        } else {
                            Rewritten::text(format!("{alias}.{name}:{id}.continue.label"), label)
                        };
                        let option = OptionDraft {
                            key: "continue".into(),
                            label,
                            visible_if: None,
                            enabled_if: None,
                            reason: None,
                            effects: Vec::new(),
                            target: next,
                        };
                        let passage = passage(
                            alias,
                            package,
                            &content,
                            vec![option],
                            texts,
                            &naming,
                            minter,
                            &path,
                        )?;
                        ("narrata.passage", passage)
                    }
                    Plan::Decision { content, choices } => {
                        let options = choices
                            .into_iter()
                            .map(|choice| OptionDraft {
                                label: Rewritten::text(
                                    format!("{alias}.{name}:{id}.{}.label", choice.id),
                                    choice.label,
                                ),
                                reason: match (&choice.enabled_if, choice.disabled_reason) {
                                    (_, Some(reason)) => Some(Rewritten::text(
                                        format!("{alias}.{name}:{id}.{}.reason", choice.id),
                                        reason,
                                    )),
                                    (Some(_), None) => Some(Rewritten::default_text(
                                        "default:disabled_reason",
                                        DEFAULT_DISABLED_REASON,
                                    )),
                                    (None, None) => None,
                                },
                                key: choice.id,
                                visible_if: choice.visible_if,
                                enabled_if: choice.enabled_if,
                                effects: choice.assignments,
                                target: choice.target,
                            })
                            .collect();
                        (
                            "narrata.passage",
                            passage(
                                alias, package, &content, options, texts, &naming, minter, &path,
                            )?,
                        )
                    }
                    Plan::Branch { .. } => ("narrata.branch", node.data.clone()),
                    Plan::Mutate { .. } => ("narrata.mutate", node.data.clone()),
                    Plan::Call { .. } => ("narrata.call", node.data.clone()),
                    Plan::Return { .. } => ("narrata.return", node.data.clone()),
                };
                nodes.insert(
                    id.clone(),
                    NodeSource {
                        id: Some(minter.node()?),
                        type_id: type_id.into(),
                        data,
                    },
                );
            }
            graphs.insert(
                name.clone(),
                GraphSource {
                    title: Some(texts.text(&format!("{alias}.{name}:title"), graph.title.clone())?),
                    parameters: graph.parameters.clone(),
                    locals: graph.locals.clone(),
                    shared: graph.shared.clone(),
                    imports: graph.imports.clone(),
                    outcomes: graph.outcomes.clone(),
                    entry: graph.entry.clone(),
                    nodes,
                },
            );
        }
        Ok(PackageSource {
            id: package.id.clone(),
            version: package.version.clone(),
            exports: package.exports.clone(),
            tombstones: Vec::new(),
            graphs,
        })
    }
}

/// A text to extract: its content key and R1 text, or a shared default.
struct Rewritten {
    key: String,
    text: String,
}

impl Rewritten {
    fn text(key: String, text: String) -> Self {
        Self { key, text }
    }

    fn default_text(key: &str, text: &str) -> Self {
        Self {
            key: key.into(),
            text: text.into(),
        }
    }
}

struct OptionDraft {
    key: String,
    label: Rewritten,
    visible_if: Option<Expr>,
    enabled_if: Option<Expr>,
    reason: Option<Rewritten>,
    effects: Vec<Assignment>,
    target: String,
}

/// An R1 `content` or `decision` node as a passage: the R1 content becomes the title and the
/// whole body, followed by one choice point `choice` whose options all branch.
#[allow(clippy::too_many_arguments)]
fn passage(
    alias: &str,
    package: &Package,
    content: &str,
    options: Vec<OptionDraft>,
    texts: &mut Texts,
    naming: &Naming,
    minter: &mut Minter,
    path: &str,
) -> Result<serde_json::Value> {
    let source = package.content.get(content).ok_or_else(|| {
        Error::new(
            "reference",
            path,
            format!("content {content} does not exist"),
        )
    })?;
    let mut reads = BTreeMap::new();
    let title = naming.rewrite(&source.title, path, &mut reads)?;
    let paragraphs = source
        .paragraphs
        .iter()
        .map(|paragraph| naming.rewrite(paragraph, path, &mut reads))
        .collect::<Result<Vec<_>>>()?;
    let title = texts.text(&format!("{alias}.{content}.title"), title)?;
    let body = texts.blocks(&format!("{alias}.{content}"), paragraphs)?;
    let mut extract =
        |text: Rewritten, reads: &mut BTreeMap<String, (Scope, String)>| -> Result<ContentRef> {
            let rewritten = naming.rewrite(&text.text, path, reads)?;
            texts.text(&text.key, rewritten)
        };
    let mut choice_options = Vec::new();
    for option in options {
        choice_options.push(OptionSource {
            id: Some(minter.option()?),
            label: Some(extract(option.label, &mut reads)?),
            reason: option
                .reason
                .map(|reason| extract(reason, &mut reads))
                .transpose()?,
            key: option.key,
            visible_if: option.visible_if,
            enabled_if: option.enabled_if,
            effects: option.effects,
            outcome: OutcomeSource::Branch {
                target: option.target,
            },
        });
    }
    let passage = PassageSource {
        title: Some(title),
        body: Some(body),
        args: reads
            .into_iter()
            .map(|(argument, (scope, name))| (argument, Expr::Read { scope, name }))
            .collect(),
        choice_points: vec![ChoicePointSource {
            id: Some(minter.choice_point()?),
            key: "choice".into(),
            placement: None,
            min: 1,
            max: 1,
            proposals: false,
            options: choice_options,
        }],
        next: None,
    };
    serde_json::to_value(passage).map_err(|e| Error::new("encoding", path, e.to_string()))
}

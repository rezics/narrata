//! The local content provider (ADR 0013 §9): JSON content packs that hold the text of a work
//! in `products/`, one pack per language. It resolves content references in batches as the
//! content-reference contract (`docs/contracts/content-references.md`) describes, formats
//! named arguments and exports content outlines.
//!
//! This is a host-side component. The node runtime never depends on it.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use narrata_kernel::{
    codec::digest_bytes,
    content::{AnchorId, ContentKey, ContentRef, ProviderId, Segment},
};
use narrata_nodes::{
    ChoicePointId, Error, Result, ViewScalar, canonical_json,
    outline::{ContentOutline, UnitOutline},
    parse_json_limited,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u16 = 1;
pub const MAX_PACK_BYTES: usize = 16 * 1024 * 1024;
/// The most items one `resolve` call accepts.
pub const MAX_BATCH: usize = 4096;
const MAX_LANGUAGE_BYTES: usize = 64;

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentPack {
    pub format_version: u16,
    pub provider: ProviderId,
    /// A BCP 47 language tag.
    pub language: String,
    pub entries: BTreeMap<ContentKey, Entry>,
}

/// A short text, or a unit of blocks that body segments address by anchor.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Entry {
    Text(TextEntry),
    Blocks(BlocksEntry),
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextEntry {
    /// `{name}` is replaced by the named argument; `{{` and `}}` are literal braces.
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlocksEntry {
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Block {
    Text {
        id: AnchorId,
        text: String,
    },
    /// Where a choice point appears in the unit; it has no text.
    Marker {
        id: AnchorId,
        choice_point: ChoicePointId,
    },
}

impl Block {
    pub fn id(&self) -> &AnchorId {
        match self {
            Self::Text { id, .. } | Self::Marker { id, .. } => id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveRequest {
    pub context: ResolveContext,
    pub items: Vec<ResolveItem>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveContext {
    /// Preferred languages, most preferred first.
    #[serde(default)]
    pub languages: Vec<String>,
    /// The host's selected translation or version; opaque to Narrata and ignored locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realization: Option<String>,
    /// The host's reader identity; opaque to Narrata and ignored locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveItem {
    pub content: Content,
    /// The values `{name}` placeholders take; `ref` values are resolved to their text.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, ViewScalar>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Content {
    Segment(Segment),
    Ref(ContentRef),
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Resolution {
    Ok {
        revision: String,
        payload: Payload,
    },
    /// Missing, withheld or withdrawn; the reason is not disclosed.
    Unavailable,
    /// Present, but an anchor, an argument or the entry shape does not fit the request.
    Incompatible {
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Payload {
    Text { text: String },
    Blocks { blocks: Vec<TextBlock> },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextBlock {
    pub id: AnchorId,
    pub text: String,
}

/// Content packs of one work. The first pack added is the original language that requests
/// fall back to.
#[derive(Clone, Debug, Default)]
pub struct LocalContent {
    packs: Vec<ContentPack>,
}

fn valid_language(tag: &str) -> bool {
    tag.len() <= MAX_LANGUAGE_BYTES
        && tag.split('-').all(|subtag| {
            (1..=8).contains(&subtag.len()) && subtag.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

impl ContentPack {
    pub fn parse(text: &str) -> Result<Self> {
        let pack: Self = parse_json_limited(text, MAX_PACK_BYTES)?;
        pack.check()?;
        Ok(pack)
    }

    pub fn check(&self) -> Result<()> {
        if self.format_version != FORMAT_VERSION {
            return Err(Error::new(
                "version",
                "format_version",
                "unsupported content pack format",
            ));
        }
        if !valid_language(&self.language) {
            return Err(Error::new(
                "language",
                "language",
                "expected a BCP 47 language tag",
            ));
        }
        for (key, entry) in &self.entries {
            if let Entry::Blocks(entry) = entry {
                let mut ids = BTreeSet::new();
                if let Some(block) = entry.blocks.iter().find(|block| !ids.insert(block.id())) {
                    return Err(Error::new(
                        "duplicate",
                        format!("entries.{key}"),
                        format!("duplicate block {}", block.id()),
                    ));
                }
            }
        }
        Ok(())
    }
}

impl LocalContent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, pack: ContentPack) -> Result<()> {
        pack.check()?;
        if self.packs.iter().any(|other| {
            other.provider == pack.provider && other.language.eq_ignore_ascii_case(&pack.language)
        }) {
            return Err(Error::new(
                "duplicate",
                "language",
                format!(
                    "a {} pack for {} is already loaded",
                    pack.provider, pack.language
                ),
            ));
        }
        self.packs.push(pack);
        Ok(())
    }

    pub fn packs(&self) -> &[ContentPack] {
        &self.packs
    }

    /// RFC 4647 lookup over the preferred languages, then the original language.
    fn pack(&self, provider: &ProviderId, languages: &[String]) -> Option<&ContentPack> {
        let candidates: Vec<&ContentPack> = self
            .packs
            .iter()
            .filter(|pack| &pack.provider == provider)
            .collect();
        for language in languages {
            let mut range = language.as_str();
            loop {
                if let Some(pack) = candidates
                    .iter()
                    .find(|pack| pack.language.eq_ignore_ascii_case(range))
                {
                    return Some(pack);
                }
                match range.rfind('-') {
                    Some(end) => {
                        range = &range[..end];
                        // A single-letter subtag never ends a range.
                        if range.len() >= 2 && range.as_bytes()[range.len() - 2] == b'-' {
                            range = &range[..range.len() - 2];
                        }
                    }
                    None => break,
                }
            }
        }
        candidates.first().copied()
    }

    /// Resolves a batch with one context, so the body and the option labels of a screen come
    /// from the same language.
    pub fn resolve(&self, request: &ResolveRequest) -> Result<Vec<Resolution>> {
        if request.items.len() > MAX_BATCH {
            return Err(Error::new(
                "limit",
                "items",
                "a batch holds at most 4096 items",
            ));
        }
        Ok(request
            .items
            .iter()
            .map(|item| self.resolve_item(item, &request.context.languages))
            .collect())
    }

    fn resolve_item(&self, item: &ResolveItem, languages: &[String]) -> Resolution {
        let (reference, first, last) = match &item.content {
            Content::Ref(reference) => (reference, None, None),
            Content::Segment(segment) => {
                (&segment.unit, segment.first.as_ref(), segment.last.as_ref())
            }
        };
        let Some(pack) = self.pack(&reference.provider, languages) else {
            return Resolution::Unavailable;
        };
        let Some(entry) = pack.entries.get(&reference.key) else {
            return Resolution::Unavailable;
        };
        let incompatible = |reason: String| Resolution::Incompatible { reason };
        let payload = match entry {
            Entry::Text(entry) => {
                if first.is_some() || last.is_some() {
                    return incompatible("a text entry has no anchors".into());
                }
                match self.format(&entry.text, &item.args, languages) {
                    Ok(text) => Payload::Text { text },
                    Err(reason) => return incompatible(reason),
                }
            }
            Entry::Blocks(entry) => {
                let position =
                    |anchor: &AnchorId| entry.blocks.iter().position(|block| block.id() == anchor);
                let start = match first.map(position) {
                    None => 0,
                    Some(Some(index)) => index,
                    Some(None) => {
                        return incompatible(format!(
                            "missing anchor {}",
                            first.map(AnchorId::as_str).unwrap_or_default()
                        ));
                    }
                };
                let end = match last.map(position) {
                    None => entry.blocks.len().saturating_sub(1),
                    Some(Some(index)) => index,
                    Some(None) => {
                        return incompatible(format!(
                            "missing anchor {}",
                            last.map(AnchorId::as_str).unwrap_or_default()
                        ));
                    }
                };
                if start > end && !entry.blocks.is_empty() {
                    return incompatible("the segment's first block follows its last".into());
                }
                let mut blocks = Vec::new();
                for block in entry.blocks.get(start..=end).unwrap_or_default() {
                    if let Block::Text { id, text } = block {
                        match self.format(text, &item.args, languages) {
                            Ok(text) => blocks.push(TextBlock {
                                id: id.clone(),
                                text,
                            }),
                            Err(reason) => return incompatible(reason),
                        }
                    }
                }
                Payload::Blocks { blocks }
            }
        };
        Resolution::Ok {
            revision: revision(pack, &reference.key, entry),
            payload,
        }
    }

    /// Replaces `{name}` with the named argument. A `ref` argument is replaced by the text it
    /// resolves to in the same languages.
    fn format(
        &self,
        text: &str,
        args: &BTreeMap<String, ViewScalar>,
        languages: &[String],
    ) -> std::result::Result<String, String> {
        let mut output = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(index) = rest.find(['{', '}']) {
            output.push_str(&rest[..index]);
            let tail = &rest[index..];
            if tail.starts_with("{{") || tail.starts_with("}}") {
                output.push_str(&tail[..1]);
                rest = &tail[2..];
                continue;
            }
            if tail.starts_with('}') {
                return Err("unmatched } (write }} for a literal brace)".into());
            }
            let end = tail
                .find('}')
                .ok_or_else(|| "unclosed { (write {{ for a literal brace)".to_owned())?;
            let name = &tail[1..end];
            let value = args
                .get(name)
                .ok_or_else(|| format!("missing argument {name}"))?;
            match value {
                ViewScalar::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
                ViewScalar::Int(value) | ViewScalar::Text(value) => output.push_str(value),
                ViewScalar::Ref(reference) => {
                    output.push_str(&self.text(reference, languages).ok_or_else(|| {
                        format!("argument {name} does not resolve to a text entry")
                    })?);
                }
            }
            rest = &tail[end + 1..];
        }
        output.push_str(rest);
        Ok(output)
    }

    /// The formatted text of a text entry without arguments, if it resolves.
    pub fn text(&self, reference: &ContentRef, languages: &[String]) -> Option<String> {
        let pack = self.pack(&reference.provider, languages)?;
        match pack.entries.get(&reference.key)? {
            Entry::Text(entry) => self.format(&entry.text, &BTreeMap::new(), languages).ok(),
            Entry::Blocks(_) => None,
        }
    }

    /// The text-free outline of the pack chosen for `languages`: every unit's blocks and the
    /// choice points marked in it.
    pub fn outline(&self, provider: &ProviderId, languages: &[String]) -> Option<ContentOutline> {
        let pack = self.pack(provider, languages)?;
        Some(outline(pack))
    }
}

pub fn outline(pack: &ContentPack) -> ContentOutline {
    let units = pack
        .entries
        .iter()
        .filter_map(|(key, entry)| match entry {
            Entry::Blocks(entry) => Some((
                key.clone(),
                UnitOutline {
                    blocks: entry
                        .blocks
                        .iter()
                        .map(|block| block.id().clone())
                        .collect(),
                    markers: entry
                        .blocks
                        .iter()
                        .filter_map(|block| match block {
                            Block::Marker { id, choice_point } => Some((id.clone(), *choice_point)),
                            Block::Text { .. } => None,
                        })
                        .collect(),
                },
            )),
            Entry::Text(_) => None,
        })
        .collect();
    ContentOutline {
        format_version: FORMAT_VERSION,
        provider: pack.provider.clone(),
        units,
    }
}

/// Changes whenever the entry's text in that language changes.
fn revision(pack: &ContentPack, key: &ContentKey, entry: &Entry) -> String {
    let bytes = canonical_json(&(&pack.language, key, entry)).unwrap_or_default();
    hex::encode(&digest_bytes("narrata.content-local.entry", 1, &bytes)[..16])
}

/// JSON Schemas of the content pack and the resolve exchange, keyed by file name.
pub fn schemas() -> Vec<(&'static str, schemars::Schema)> {
    use schemars::schema_for;
    vec![
        ("content-pack.schema.json", schema_for!(ContentPack)),
        (
            "content-resolve-request.schema.json",
            schema_for!(ResolveRequest),
        ),
        (
            "content-resolution.schema.json",
            schema_for!(Vec<Resolution>),
        ),
    ]
}

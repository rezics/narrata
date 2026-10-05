//! Deterministic upgrade from Program format 0, which carries reader text, to the text-free
//! format 1 (ADR 0018 §5). The text moves into a local content pack that the host resolves;
//! saves move with the ADR 0009 migration that [`format_upgrade_migration`] describes.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    MigrationId, Value,
    codec::digest_bytes,
    migration::{
        DeclarativeMigration, MigrationDescriptor, MigrationError, RelocationTable, VersionRange,
    },
    program::{
        CheckedProgram, ConstIndex, ContentEntryV1, ContentError, ContentIndex, ContentOperand,
        ContentRef, OpV0, ProgramArtifactV0, Segment,
    },
    version::{PROGRAM_FORMAT_V0, PROGRAM_FORMAT_V1, SNAPSHOT_SCHEMA_V0},
};

/// The provider of every reference an upgrade creates: the local content provider of
/// ADR 0013 §9.
pub const UPGRADE_CONTENT_PROVIDER: &str = "local";
/// The pack language when the caller does not know it (BCP 47 "undetermined").
pub const UNDETERMINED_LANGUAGE: &str = "und";
const LOCAL_PACK_FORMAT: u16 = 1;
const MAX_LANGUAGE_BYTES: usize = 64;

/// The text entries of a local content pack (ADR 0013 §9, format 1): what an upgrade moves out
/// of a format 0 Program. Braces are doubled because the local provider reads `{name}` as a
/// placeholder.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalContentPack {
    pub format_version: u16,
    pub provider: String,
    pub language: String,
    pub entries: BTreeMap<String, LocalTextEntry>,
}

impl LocalContentPack {
    pub fn new(language: &str) -> Self {
        Self {
            format_version: LOCAL_PACK_FORMAT,
            provider: UPGRADE_CONTENT_PROVIDER.to_owned(),
            language: language.to_owned(),
            entries: BTreeMap::new(),
        }
    }

    /// The JSON the local content provider reads.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalTextEntry {
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct UpgradedProgram {
    pub artifact: ProgramArtifactV0,
    pub content: LocalContentPack,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum UpgradeError {
    #[error("only Program format 0 upgrades to format 1")]
    NotFormatV0,
    #[error("presentation operand names constant {0}, which is not a string")]
    NotText(u32),
    #[error("two different texts map to content key {0}")]
    KeyCollision(String),
    #[error("content reference is invalid: {0}")]
    Content(#[from] ContentError),
    #[error("content pack language must be a BCP 47 tag")]
    Language,
    #[error("the content table would exceed u32 indices")]
    TooManyEntries,
}

/// The reference a format 0 text becomes. The same text always gets the same reference, so packs
/// from upgrades of different artifacts merge without conflict.
pub fn upgraded_text_reference(text: &str) -> Result<ContentRef, ContentError> {
    let digest = digest_bytes("narrata.stage5-text", 1, text.as_bytes());
    ContentRef::new(
        UPGRADE_CONTENT_PROVIDER,
        &format!("stage5-{}", hex::encode(&digest[..16])),
    )
}

/// Rewrites a checked format 0 Program as format 1 plus the content pack its text moves to.
pub fn upgrade_program_v0(
    program: &CheckedProgram,
    language: &str,
) -> Result<UpgradedProgram, UpgradeError> {
    let source = program.artifact();
    if source.format_version != PROGRAM_FORMAT_V0 {
        return Err(UpgradeError::NotFormatV0);
    }
    if !valid_language(language) {
        return Err(UpgradeError::Language);
    }
    let kept = kept_constants(source);
    let mut table = ContentTable::default();
    let mut artifact = source.clone();
    artifact.format_version = PROGRAM_FORMAT_V1;
    artifact.constants = kept
        .keys()
        .filter_map(|index| source.constants.get(*index as usize).cloned())
        .collect();
    for flow in &mut artifact.flows {
        for instruction in &mut flow.instructions {
            match &mut instruction.op {
                OpV0::Const { constant, .. } => {
                    *constant = ConstIndex(kept.get(&constant.0).copied().unwrap_or(constant.0));
                }
                OpV0::Say { speaker, text, .. } => {
                    if let Some(speaker) = speaker {
                        *speaker = table.add(program, *speaker, Role::Reference)?;
                    }
                    *text = table.add(program, *text, Role::Segment)?;
                }
                OpV0::Choice { prompt, choices } => {
                    if let Some(prompt) = prompt {
                        *prompt = table.add(program, *prompt, Role::Reference)?;
                    }
                    for choice in choices {
                        choice.label = table.add(program, choice.label, Role::Reference)?;
                    }
                }
                _ => {}
            }
        }
    }
    artifact.content = table.entries;
    Ok(UpgradedProgram {
        artifact,
        content: LocalContentPack {
            entries: table.texts,
            ..LocalContentPack::new(language)
        },
    })
}

/// The ADR 0009 migration from a format 0 Program to its upgrade: no relocations, no recovery
/// points, Snapshot schema 0 only. `target` must be exactly what [`upgrade_program_v0`] makes
/// of `source`; the content pack language does not affect it.
pub fn format_upgrade_migration(
    source: &CheckedProgram,
    target: &CheckedProgram,
) -> Result<DeclarativeMigration, MigrationError> {
    let expected = upgrade_program_v0(source, UNDETERMINED_LANGUAGE)
        .map_err(|_| MigrationError::ArtifactMismatch)?;
    if target.format_version() != PROGRAM_FORMAT_V1 || target.artifact() != &expected.artifact {
        return Err(MigrationError::ArtifactMismatch);
    }
    let mut seed = Vec::with_capacity(64);
    seed.extend_from_slice(source.artifact_id().as_bytes());
    seed.extend_from_slice(target.artifact_id().as_bytes());
    let digest = digest_bytes("narrata.stage5-format-upgrade", 1, &seed);
    let mut id = [0_u8; 16];
    id.copy_from_slice(&digest[..16]);
    DeclarativeMigration::new(MigrationDescriptor {
        id: MigrationId::from_bytes(id),
        from: source.artifact_id(),
        to: target.artifact_id(),
        accepted_snapshot_schemas: VersionRange {
            minimum: SNAPSHOT_SCHEMA_V0.get(),
            maximum: SNAPSHOT_SCHEMA_V0.get(),
        },
        relocations: RelocationTable::default(),
        recovery_points: BTreeMap::new(),
        recovery_locations: BTreeMap::new(),
    })
}

/// Constants a `Const` instruction pushes, mapped to their new index. Text used only for
/// presentation leaves the Program.
fn kept_constants(artifact: &ProgramArtifactV0) -> BTreeMap<u32, u32> {
    let used = artifact
        .flows
        .iter()
        .flat_map(|flow| &flow.instructions)
        .filter_map(|instruction| match instruction.op {
            OpV0::Const { constant, .. } => Some(constant.0),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    used.into_iter()
        .zip(0_u32..)
        .collect::<BTreeMap<u32, u32>>()
}

#[derive(Clone, Copy)]
enum Role {
    Reference,
    Segment,
}

#[derive(Default)]
struct ContentTable {
    entries: Vec<ContentEntryV1>,
    indices: BTreeMap<ContentEntryV1, u32>,
    texts: BTreeMap<String, LocalTextEntry>,
}

impl ContentTable {
    /// Entries are numbered by first use; each distinct entry appears once.
    fn add(
        &mut self,
        program: &CheckedProgram,
        operand: ContentOperand,
        role: Role,
    ) -> Result<ContentOperand, UpgradeError> {
        let ContentOperand::Constant(constant) = operand else {
            return Err(UpgradeError::NotFormatV0);
        };
        let Some(Value::String(text)) = program.constant(constant) else {
            return Err(UpgradeError::NotText(constant.0));
        };
        let reference = upgraded_text_reference(text)?;
        let escaped = text.replace('{', "{{").replace('}', "}}");
        match self.texts.get(reference.key.as_str()) {
            Some(existing) if existing.text != escaped => {
                return Err(UpgradeError::KeyCollision(reference.key.to_string()));
            }
            Some(_) => {}
            None => {
                self.texts
                    .insert(reference.key.to_string(), LocalTextEntry { text: escaped });
            }
        }
        let entry = match role {
            Role::Reference => ContentEntryV1::Ref(reference),
            Role::Segment => ContentEntryV1::Segment(Segment::unit(reference)),
        };
        let next = u32::try_from(self.entries.len()).map_err(|_| UpgradeError::TooManyEntries)?;
        let index = *self.indices.entry(entry.clone()).or_insert_with(|| {
            self.entries.push(entry);
            next
        });
        Ok(ContentOperand::Content(ContentIndex(index)))
    }
}

/// The local provider's language rule: dash-separated alphanumeric subtags of 1–8 bytes.
fn valid_language(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= MAX_LANGUAGE_BYTES
        && tag.split('-').all(|subtag| {
            (1..=8).contains(&subtag.len()) && subtag.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

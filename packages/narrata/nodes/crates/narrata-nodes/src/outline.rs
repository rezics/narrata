use std::collections::BTreeMap;

use narrata_kernel::content::{AnchorId, ContentKey, ProviderId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ChoicePointId;

/// A content provider's outline (ADR 0013 §4): block order and choice-point markers of each
/// content unit, without text. It lets `compose` check placement order without the documents.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentOutline {
    pub format_version: u16,
    pub provider: ProviderId,
    pub units: BTreeMap<ContentKey, UnitOutline>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnitOutline {
    /// Every block anchor in document order, markers included.
    pub blocks: Vec<AnchorId>,
    /// Marker blocks and the choice point each one stands for.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub markers: BTreeMap<AnchorId, ChoicePointId>,
}

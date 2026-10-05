use std::collections::{BTreeMap, BTreeSet};

use narrata_kernel::content::{AnchorId, ContentKey, ContentRef, ProviderId, Segment};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ChoicePointId, Diagnostic, Error, NameTable, Program, Result,
    plan::{Outcome, Passage, Plan, node_path},
};

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

fn diagnostic(out: &mut Vec<Diagnostic>, code: &str, path: &str, message: impl Into<String>) {
    out.push(Diagnostic {
        code: code.into(),
        path: path.into(),
        message: message.into(),
    });
}

struct Positions<'a> {
    blocks: BTreeMap<&'a AnchorId, usize>,
    len: usize,
}

impl Positions<'_> {
    fn anchor(&self, anchor: &AnchorId, path: &str, out: &mut Vec<Diagnostic>) -> Option<usize> {
        let position = self.blocks.get(anchor).copied();
        if position.is_none() {
            diagnostic(
                out,
                "outline_anchor_missing",
                path,
                format!("Anchor {anchor} is absent from the content outline."),
            );
        }
        position
    }

    fn segment(
        &self,
        segment: &Segment,
        path: &str,
        out: &mut Vec<Diagnostic>,
    ) -> Option<(usize, usize)> {
        let first = match &segment.first {
            Some(anchor) => self.anchor(anchor, &format!("{path}.first"), out),
            None => (self.len > 0).then_some(0),
        };
        let last = match &segment.last {
            Some(anchor) => self.anchor(anchor, &format!("{path}.last"), out),
            None => self.len.checked_sub(1),
        };
        if self.len == 0 {
            diagnostic(
                out,
                "outline_empty_unit",
                path,
                "The content unit has no blocks.",
            );
        }
        let (first, last) = (first?, last?);
        if first > last {
            diagnostic(
                out,
                "outline_order",
                path,
                "The segment's first block follows its last block.",
            );
            return None;
        }
        Some((first, last))
    }

    fn passage(&self, passage: &Passage, body: &Segment, path: &str, out: &mut Vec<Diagnostic>) {
        let range = self.segment(body, &format!("{path}.body"), out);
        let placements: Vec<_> = passage
            .choice_points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let point_path = format!("{path}.choice_points[{index}].placement");
                match &point.placement {
                    Some(anchor) => {
                        let position = self.anchor(anchor, &point_path, out);
                        if let (Some(position), Some((first, last))) = (position, range)
                            && !(first..=last).contains(&position)
                        {
                            diagnostic(
                                out,
                                "outline_order",
                                &point_path,
                                "The placement is outside the body slice.",
                            );
                        }
                        position
                    }
                    // An omitted final placement is after the slice, not after the entire unit.
                    None => range.map(|(_, last)| last + 1),
                }
            })
            .collect();
        for (index, point) in passage.choice_points.iter().enumerate() {
            let point_path = format!("{path}.choice_points[{index}]");
            let placement = placements[index];
            let next = placements.get(index + 1).copied().flatten();
            if let (Some(placement), Some(next)) = (placement, next)
                && placement > next
            {
                diagnostic(
                    out,
                    "outline_order",
                    &point_path,
                    "The next placement precedes this placement.",
                );
            }
            for (option_index, option) in point.options.iter().enumerate() {
                let Outcome::Local { reply, rejoin } = &option.outcome else {
                    continue;
                };
                let option_path = format!("{point_path}.options[{option_index}].outcome");
                let rejoin_position = match rejoin {
                    Some(anchor) => self.anchor(anchor, &format!("{option_path}.rejoin"), out),
                    None => next.or_else(|| range.map(|(_, last)| last + 1)),
                };
                if let (Some(position), Some((first, last))) = (rejoin_position, range) {
                    if rejoin.is_some() && !(first..=last).contains(&position) {
                        diagnostic(
                            out,
                            "outline_order",
                            &option_path,
                            "The rejoin is outside the body slice.",
                        );
                    }
                    if placement.is_some_and(|placement| position < placement)
                        || (rejoin.is_some() && next.is_some_and(|next| position > next))
                    {
                        diagnostic(
                            out,
                            "outline_order",
                            &option_path,
                            "The rejoin must follow placement and precede or equal the next placement.",
                        );
                    }
                }
                if let Some(reply) = reply {
                    let reply_range = self.segment(reply, &format!("{option_path}.reply"), out);
                    if let (Some((first, last)), Some((body_first, body_last))) =
                        (reply_range, range)
                        && (first < body_first
                            || last > body_last
                            || placement.is_some_and(|placement| first <= placement)
                            || rejoin_position.is_some_and(|rejoin| last >= rejoin))
                    {
                        diagnostic(
                            out,
                            "outline_order",
                            &option_path,
                            "The reply must lie inside the body slice and strictly between placement and rejoin (or the end of the slice).",
                        );
                    }
                }
            }
        }
    }
}

/// Pure publication-time checks. Content defects are diagnostics; an unsupported outline
/// version or an unreadable program is an error. Only this outline's provider is inspected.
pub fn diagnose_outline(
    program: &Program,
    names: Option<&NameTable>,
    outline: &ContentOutline,
) -> Result<Vec<Diagnostic>> {
    if outline.format_version != 1 {
        return Err(Error::new(
            "outline_version",
            "format_version",
            "expected content outline version 1",
        ));
    }
    let mut out = Vec::new();
    let mut expected = BTreeMap::new();
    let mut passages = BTreeMap::<_, Vec<_>>::new();
    for reference in program.manifest().graphs.keys() {
        let graph = program.graph(reference)?;
        for (id, plan) in &graph.nodes {
            if let Plan::Passage(passage) = plan {
                let path = node_path(names, reference, id);
                for point in &passage.choice_points {
                    expected.insert(
                        point.id,
                        (
                            passage.body.as_ref().map(|body| &body.unit).cloned(),
                            point.placement.clone(),
                        ),
                    );
                }
                if let Some(body) = &passage.body
                    && body.unit.provider == outline.provider
                {
                    passages.entry(body.unit.key.clone()).or_default().push((
                        passage.clone(),
                        body.clone(),
                        path,
                    ));
                }
            }
        }
    }
    let mut marker_points = BTreeSet::new();
    for (key, unit) in &outline.units {
        let unit_ref = ContentRef {
            provider: outline.provider.clone(),
            key: key.clone(),
        };
        let path = format!("outline.{}.{}", outline.provider, key);
        let mut blocks = BTreeMap::new();
        for (index, anchor) in unit.blocks.iter().enumerate() {
            if blocks.insert(anchor, index).is_some() {
                diagnostic(
                    &mut out,
                    "outline_duplicate_anchor",
                    &path,
                    format!("Anchor {anchor} appears more than once."),
                );
            }
        }
        let positions = Positions {
            blocks,
            len: unit.blocks.len(),
        };
        for (anchor, point) in &unit.markers {
            positions.anchor(anchor, &path, &mut out);
            if !marker_points.insert(*point) {
                diagnostic(
                    &mut out,
                    "outline_duplicate_marker",
                    &path,
                    format!("Choice point {point} has more than one marker."),
                );
            }
            match expected.get(point) {
                None => diagnostic(
                    &mut out,
                    "outline_unknown_marker",
                    &path,
                    format!("Marker {anchor} refers to unknown choice point {point}."),
                ),
                Some((body, placement))
                    if body.as_ref() != Some(&unit_ref) || placement.as_ref() != Some(anchor) =>
                {
                    diagnostic(
                        &mut out,
                        "outline_marker_mismatch",
                        &path,
                        format!(
                            "Marker {anchor} disagrees with choice point {point}'s body unit or placement."
                        ),
                    );
                }
                Some(_) => {}
            }
        }
        if let Some(unit_passages) = passages.remove(key) {
            for (passage, body, node_path) in unit_passages {
                positions.passage(&passage, &body, &node_path, &mut out);
            }
        }
    }
    for unit_passages in passages.into_values() {
        for (_, body, path) in unit_passages {
            diagnostic(
                &mut out,
                "outline_unit_missing",
                &path,
                format!("Content unit {} is absent from the outline.", body.unit.key),
            );
        }
    }
    Ok(out)
}

//! What a cache knows about one ordered byte-keyed space: entries it holds, entries it knows
//! are absent, and ranges it holds completely.

use std::{collections::BTreeMap, ops::Bound};

/// Entries keyed by bytes, `None` for an entry known to be absent, plus the ranges in which
/// every existing entry is held. A key that is neither held nor covered is unknown.
#[derive(Debug)]
pub(crate) struct Known<V> {
    entries: BTreeMap<Vec<u8>, Option<V>>,
    coverage: Coverage,
}

impl<V> Default for Known<V> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            coverage: Coverage::default(),
        }
    }
}

/// Present entries found by [`Known::walk`], and where knowledge ran out before enough were
/// found and before the end of the walk.
pub(crate) struct Walk<'a, V> {
    pub found: Vec<(&'a [u8], &'a V)>,
    pub gap: Option<Vec<u8>>,
}

impl<V> Known<V> {
    /// `Some(None)` for a key known to be absent, `None` for an unknown key.
    pub fn get(&self, key: &[u8]) -> Option<Option<&V>> {
        match self.entries.get(key) {
            Some(value) => Some(value.as_ref()),
            None if self.coverage.contains(key) => Some(None),
            None => None,
        }
    }

    /// Records what a write did; the entry is known from now on.
    pub fn set(&mut self, key: Vec<u8>, value: Option<V>) {
        self.entries.insert(key, value);
    }

    /// Records what a host reported, unless the cache already knows the key: what it knows
    /// is the host's state plus the batches the host has not yet persisted.
    pub fn learn(&mut self, key: &[u8], value: Option<V>) {
        if self.get(key).is_none() {
            self.entries.insert(key.to_vec(), value);
        }
    }

    pub fn cover(&mut self, start: Vec<u8>, end: Option<Vec<u8>>) {
        self.coverage.insert(start, end);
    }

    /// Knows that nothing exists outside what it holds, as for a store never written to.
    pub fn cover_all(&mut self) {
        self.coverage.insert(Vec::new(), None);
    }

    /// Present entries from `lower` (inclusive) towards `end` (exclusive, or unbounded),
    /// stopping after `need` of them or where knowledge runs out.
    pub fn walk(&self, lower: &[u8], end: Option<&[u8]>, need: usize) -> Walk<'_, V> {
        let mut found = Vec::new();
        let mut position = lower.to_vec();
        loop {
            if found.len() >= need || end.is_some_and(|end| position.as_slice() >= end) {
                return Walk { found, gap: None };
            }
            let Some(extent) = self.coverage.extent(&position) else {
                return Walk {
                    found,
                    gap: Some(position),
                };
            };
            let stop = match (extent, end) {
                (Some(extent), Some(end)) => Some(extent.min(end)),
                (extent, end) => extent.or(end),
            };
            let upper = stop.map_or(Bound::Unbounded, Bound::Excluded);
            let entries = self
                .entries
                .range::<[u8], _>((Bound::Included(position.as_slice()), upper));
            for (key, value) in entries {
                if let Some(value) = value {
                    found.push((key.as_slice(), value));
                    if found.len() == need {
                        return Walk { found, gap: None };
                    }
                }
            }
            match extent {
                Some(extent) => position = extent.to_vec(),
                None => return Walk { found, gap: None },
            }
        }
    }
}

/// Disjoint, non-adjacent half-open intervals `[start, end)`; `None` ends at infinity.
#[derive(Debug, Default)]
struct Coverage(BTreeMap<Vec<u8>, Option<Vec<u8>>>);

impl Coverage {
    /// The end of the interval holding `point`, if one does.
    fn extent(&self, point: &[u8]) -> Option<Option<&[u8]>> {
        let (_, end) = self
            .0
            .range::<[u8], _>((Bound::Unbounded, Bound::Included(point)))
            .next_back()?;
        match end {
            None => Some(None),
            Some(end) if point < end.as_slice() => Some(Some(end.as_slice())),
            Some(_) => None,
        }
    }

    fn contains(&self, point: &[u8]) -> bool {
        self.extent(point).is_some()
    }

    fn insert(&mut self, mut start: Vec<u8>, mut end: Option<Vec<u8>>) {
        if end.as_ref().is_some_and(|end| *end <= start) {
            return;
        }
        if let Some((before, before_end)) = self
            .0
            .range::<[u8], _>((Bound::Unbounded, Bound::Included(start.as_slice())))
            .next_back()
            && before_end
                .as_ref()
                .is_none_or(|before_end| *before_end >= start)
        {
            start = before.clone();
        }
        let upper = end
            .as_ref()
            .map_or(Bound::Unbounded, |end| Bound::Included(end.clone()));
        let absorbed = self
            .0
            .range((Bound::Included(start.clone()), upper))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in absorbed {
            if let Some(other) = self.0.remove(&key) {
                end = match (end, other) {
                    (Some(end), Some(other)) => Some(end.max(other)),
                    _ => None,
                };
            }
        }
        self.0.insert(start, end);
    }
}

/// The first byte string after every string that starts with `prefix`; `None` when there is
/// none, as for an empty prefix or one of only `0xFF` bytes.
pub(crate) fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last < u8::MAX {
            end.push(last + 1);
            return Some(end);
        }
    }
    None
}

/// The first byte string after `key`.
pub(crate) fn successor(key: &[u8]) -> Vec<u8> {
    let mut next = Vec::with_capacity(key.len() + 1);
    next.extend_from_slice(key);
    next.push(0);
    next
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::BTreeSet;

    use proptest::prelude::*;

    use super::*;

    fn key() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(prop::sample::select(vec![0_u8, 1, 2, 0xFF]), 0..4)
    }

    proptest! {
        /// Inserting intervals in any order covers exactly the points some interval covers.
        #[test]
        fn coverage_matches_the_union_of_its_intervals(
            intervals in prop::collection::vec((key(), prop::option::of(key())), 0..8),
            points in prop::collection::vec(key(), 0..16),
        ) {
            let mut coverage = Coverage::default();
            for (start, end) in &intervals {
                coverage.insert(start.clone(), end.clone());
            }
            for point in &points {
                let expected = intervals.iter().any(|(start, end)| {
                    point >= start && end.as_ref().is_none_or(|end| point < end)
                });
                prop_assert_eq!(coverage.contains(point), expected, "point {:?}", point);
            }
            let starts = coverage.0.keys().collect::<Vec<_>>();
            for pair in coverage.0.iter().collect::<Vec<_>>().windows(2) {
                let (_, end) = pair[0];
                let (next, _) = pair[1];
                prop_assert!(end.as_ref().is_some_and(|end| end < next), "{starts:?}");
            }
        }

        /// A walk returns the present entries a full scan would, or stops at the first unknown
        /// position.
        #[test]
        fn walks_stop_where_knowledge_does(
            present in prop::collection::btree_set(key(), 0..8),
            covered in prop::collection::vec((key(), prop::option::of(key())), 0..4),
            lower in key(),
            end in prop::option::of(key()),
            need in 1_usize..4,
        ) {
            let mut known = Known::default();
            for (start, stop) in &covered {
                known.cover(start.clone(), stop.clone());
            }
            for key in &present {
                if known.get(key).is_some() {
                    known.set(key.clone(), Some(()));
                }
            }
            let walk = known.walk(&lower, end.as_deref(), need);
            let in_range = |key: &Vec<u8>| {
                *key >= lower && end.as_ref().is_none_or(|end| key < end)
            };
            let held = present
                .iter()
                .filter(|key| in_range(key) && known.get(key) == Some(Some(&())))
                .collect::<BTreeSet<_>>();
            let found = walk.found.iter().map(|(key, _)| key.to_vec()).collect::<Vec<_>>();
            match &walk.gap {
                Some(gap) => {
                    prop_assert!(!known.coverage.contains(gap) && in_range(gap));
                    prop_assert!(found.len() < need);
                    let before = held.iter().filter(|key| **key < gap).cloned().cloned().collect::<Vec<_>>();
                    prop_assert_eq!(found, before);
                }
                None => {
                    let expected = held.iter().take(need).cloned().cloned().collect::<Vec<_>>();
                    prop_assert_eq!(found, expected);
                }
            }
        }
    }

    #[test]
    fn byte_string_successors() {
        assert_eq!(prefix_end(b""), None);
        assert_eq!(prefix_end(&[0xFF, 0xFF]), None);
        assert_eq!(prefix_end(&[1, 0xFF]), Some(vec![2]));
        assert_eq!(prefix_end(&[1, 2]), Some(vec![1, 3]));
        assert_eq!(successor(&[1]), vec![1, 0]);
    }
}

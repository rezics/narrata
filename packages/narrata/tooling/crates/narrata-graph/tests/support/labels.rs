#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use narrata_graph::{ClusterId, Id, LabelTable};
use narrata_kernel::content::ContentRef;

pub fn tables() -> Vec<LabelTable> {
    vec![
        LabelTable {
            cluster: ClusterId::from_u128(1),
            titles: BTreeMap::from([
                (
                    Id::from_u128(2),
                    ContentRef::new("local", "山路:title").expect("valid reference"),
                ),
                (
                    Id::from_u128(3),
                    ContentRef::new("rezics", "book/entry:title").expect("valid reference"),
                ),
            ]),
        },
        LabelTable {
            cluster: ClusterId::from_u128(4),
            titles: BTreeMap::new(),
        },
    ]
}

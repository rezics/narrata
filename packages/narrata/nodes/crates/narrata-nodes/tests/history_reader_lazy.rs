#![allow(clippy::unwrap_used)]

use narrata_nodes::{ChunkSource, Error, ObjectId, Pack, Program, Session};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Offline(Arc<AtomicUsize>);
impl ChunkSource for Offline {
    fn load(&self, id: &ObjectId) -> narrata_nodes::Result<Vec<u8>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Error::new("not_loaded", id.to_string(), "offline"))
    }
}

#[test]
fn history_registration_checks_metadata_without_fetching_unread_chunks() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat/nodes-evolution-v1/republished-after-migration");
    let (current, _) =
        Program::from_pack(&std::fs::read(root.join("target.narpack")).unwrap()).unwrap();
    let mut session = Session::new(
        Arc::new(current),
        "execution:0190f2a0000070008000000000000001"
            .parse()
            .unwrap(),
    )
    .unwrap();
    let old = Pack::decode(&std::fs::read(root.join("ancestor.narpack")).unwrap()).unwrap();
    let loads = Arc::new(AtomicUsize::new(0));
    let historic =
        Arc::new(Program::from_manifest(&old.manifest, Box::new(Offline(loads.clone()))).unwrap());
    session.register_history_program(historic.clone()).unwrap();
    assert_eq!(loads.load(Ordering::SeqCst), 0);
    // Lazy registration does not claim the chunks are present/valid. The checked read
    // remains unavailable and can be retried when the host supplies its exact object.
    assert_eq!(historic.verify_artifact().unwrap_err().code, "not_loaded");
    assert_eq!(loads.load(Ordering::SeqCst), 1);
}

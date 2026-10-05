#![allow(clippy::unwrap_used)]

use narrata_authoring::publication::{publication_from_pack, read_publication};
use narrata_node_tools::publish::write_object_collection;
use std::{fs, path::Path};

#[test]
fn cli_objects_are_idempotent_checked_on_disk_and_never_overwrite_conflicting_objects() {
    let input = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../products/gamebook-demo/story.narpack");
    let publication = publication_from_pack(&fs::read(&input).unwrap()).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let description = temp.path().join("publication.json");
    let objects = temp.path().join("objects");
    let arguments = vec![
        "publication".into(),
        input.display().to_string(),
        "--out".into(),
        description.display().to_string(),
        "--objects".into(),
        objects.display().to_string(),
    ];
    narrata_node_tools::run(&arguments).unwrap();
    narrata_node_tools::run(&arguments).unwrap();
    assert_eq!(
        fs::read(&description).unwrap(),
        publication.description_bytes()
    );
    read_publication(
        &fs::read(&description).unwrap(),
        publication.publication_id(),
        |id| {
            let reference = publication
                .description()
                .objects
                .iter()
                .find(|r| r.object_id == id)
                .unwrap();
            narrata_node_tools::read_bytes(&objects.join(reference.filename()))
        },
    )
    .unwrap();
    let object = &publication.description().objects[0];
    fs::write(objects.join(object.filename()), b"damaged immutable object").unwrap();
    assert_eq!(
        write_object_collection(&description, &objects, &publication)
            .unwrap_err()
            .code,
        "object_conflict"
    );
    assert_eq!(
        fs::read(&description).unwrap(),
        publication.description_bytes()
    );
    assert_eq!(
        fs::read(objects.join(object.filename())).unwrap(),
        b"damaged immutable object"
    );
}

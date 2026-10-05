#![allow(clippy::unwrap_used)]

mod support;

use narrata_kernel::codec::digest_bytes;
use narrata_nodes::{ProposalRequest, Session};
use serde_json::json;
use support::*;

fn snapshot(session: &Session, bytes: &mut Vec<Vec<u8>>) {
    bytes.push(session.state().unwrap().encode());
    bytes.push(session.state().unwrap().envelope());
    bytes.push(session.state().unwrap().id().as_bytes().to_vec());
    bytes.push(session.cursor().unwrap().as_bytes().to_vec());
    bytes.push(serde_json::to_vec(&session.view(None).unwrap()).unwrap());
    for (_, commit) in session.commits().unwrap() {
        bytes.push(commit.envelope());
    }
}

#[test]
fn session_bytes_match_with_and_without_the_trace_feature() {
    let mut bytes = Vec::new();
    let compilation = compiled();
    let (mut book, names) = session(&compilation);
    snapshot(&book, &mut bytes);
    for keys in [
        &["wave"][..],
        &["rope", "lamp"][..],
        &["left"][..],
        &["continue"][..],
    ] {
        choose(&mut book, &names, keys).unwrap();
        snapshot(&book, &mut bytes);
    }

    let compilation = try_variant(|_, main, _| {
        set(
            main,
            &format!("{GATE}/choice_points/0"),
            "proposals",
            json!(true),
        );
    })
    .unwrap();
    let (mut book, _) = session(&compilation);
    let request: ProposalRequest = serde_json::from_value(json!({
        "choice_point": point(1),
        "options": [{"label": content("detour"), "outcome": {"kind": "branch", "target": "detour"}}],
        "nodes": [{"key": "detour", "body": {"unit": content("detour")}, "next": node(4)}]
    })).unwrap();
    let parent = book.cursor().unwrap();
    book.propose(&parent, &request).unwrap();
    snapshot(&book, &mut bytes);
    let proposed = book.state().unwrap().frames[0]
        .overlay
        .options
        .values()
        .next()
        .unwrap()[0]
        .id;
    book.choose(
        &book.cursor().unwrap(),
        point(1).parse().unwrap(),
        vec![proposed],
    )
    .unwrap();
    snapshot(&book, &mut bytes);

    // One expected digest is checked by both Cargo configurations; trace-enabled execution
    // is also compared directly against these ordinary sessions in the observation tests.
    let digest = hex::encode(digest_bytes(
        "test.session-trace-parity",
        1,
        &serde_json::to_vec(&bytes).unwrap(),
    ));
    assert_eq!(
        digest,
        "cd5ac4b95b5139381eb362bee6d4203562ea2fc37be40d8a5cd44b0e3c3a19e3"
    );
}

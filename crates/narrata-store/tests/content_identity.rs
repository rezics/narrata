#![allow(clippy::panic, clippy::unwrap_used)]

//! Decision 2 and ADR 0018: editing the text in a content pack changes no Program identity,
//! state digest, Snapshot, Receipt or Commit; changing a reference's provider or key does.

use std::sync::Arc;

use narrata_core::{
    ChoiceId, CommitId, ExecutionId, InputId, ProgramArtifactId, ReceiptId, StateDigest,
    program::{
        ContentEntryV1, ContentRef, ProgramArtifactV0, encode_program_artifact, load_program,
    },
    runtime::{CheckedRuntimeInput, ContentView, DraftResult, PendingInteractionV0},
    snapshot::{export_snapshot, state_digest},
    upgrade::LocalContentPack,
};
use narrata_store::{BranchId, InitialRecordingMode, MemoryStore, RefName, SessionCoordinator};
use narrata_testkit::generator::{branch_call_choice_v1, story_content};
use proptest::prelude::*;

#[derive(Debug, Eq, PartialEq)]
struct Run {
    artifact: ProgramArtifactId,
    commits: Vec<CommitId>,
    receipts: Vec<ReceiptId>,
    states: Vec<StateDigest>,
    snapshots: Vec<Vec<u8>>,
    /// Every reference the run presented, in order.
    presented: Vec<ContentRef>,
}

/// Plays the story through the store: Start, Advance, select the first choice, Advance.
fn run(artifact: &ProgramArtifactV0) -> Run {
    let program = load_program(&encode_program_artifact(artifact), &Default::default()).unwrap();
    let mut coordinator = SessionCoordinator::create(
        MemoryStore::new(),
        Arc::clone(&program),
        ExecutionId::from_u128(70),
        RefName::new("identity").unwrap(),
        BranchId::from_u128(70),
        InitialRecordingMode::Complete,
        1,
    )
    .unwrap();
    let mut output = Run {
        artifact: program.artifact_id(),
        commits: vec![coordinator.timeline().cursor],
        receipts: Vec::new(),
        states: vec![state_digest(coordinator.state())],
        snapshots: vec![export_snapshot(coordinator.state()).unwrap()],
        presented: Vec::new(),
    };
    let mut input = CheckedRuntimeInput::start(InputId::from_u128(1));
    for request in 2_u128..10 {
        let committed = coordinator.dispatch(input, Default::default(), 2).unwrap();
        output.commits.push(committed.commit);
        output.receipts.push(committed.receipt);
        output.states.push(state_digest(&committed.state));
        output
            .snapshots
            .push(export_snapshot(&committed.state).unwrap());
        let views: Vec<&ContentView> = match &committed.result {
            DraftResult::AwaitSay(view) => view.speaker.iter().chain([&view.text]).collect(),
            DraftResult::AwaitChoice(view) => view
                .prompt
                .iter()
                .chain(view.choices.iter().map(|choice| &choice.label))
                .collect(),
            DraftResult::Finished(_) => return output,
            other => panic!("unexpected result {other:?}"),
        };
        output
            .presented
            .extend(views.into_iter().map(|view| match view {
                ContentView::Ref(reference) => reference.clone(),
                ContentView::Segment(segment) => segment.unit.clone(),
                ContentView::LegacyText(_) => panic!("format 1 never presents text"),
            }));
        let request = InputId::from_u128(request);
        input = match committed.state.pending().unwrap() {
            PendingInteractionV0::Say { interaction_id, .. } => {
                CheckedRuntimeInput::advance(request, *interaction_id)
            }
            PendingInteractionV0::Choice { interaction_id, .. } => {
                CheckedRuntimeInput::select(request, *interaction_id, ChoiceId::from_u128(1))
            }
        };
    }
    panic!("the story did not finish");
}

fn with_texts(texts: &[String]) -> LocalContentPack {
    let mut pack = story_content();
    for (entry, text) in pack.entries.values_mut().zip(texts) {
        entry.text.clone_from(text);
    }
    pack
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn text_in_the_content_pack_never_reaches_an_identity(
        texts in prop::collection::vec(any::<String>(), 9)
    ) {
        let baseline = run(&branch_call_choice_v1());
        let story = branch_call_choice_v1();
        let pack = with_texts(&texts);
        prop_assert_eq!(&run(&story), &baseline);
        // The host still shows the edited text: every presented key resolves in the new pack.
        for reference in &baseline.presented {
            let original = &story_content().entries[reference.key.as_str()].text;
            let index = story_content()
                .entries
                .keys()
                .position(|key| key == reference.key.as_str())
                .unwrap();
            prop_assert_eq!(&pack.entries[reference.key.as_str()].text, &texts[index]);
            prop_assert!(!original.is_empty());
        }
    }

    #[test]
    fn a_new_provider_or_key_changes_every_identity(
        entry in 0_usize..7,
        provider in "[a-z0-9-]{1,12}",
        key in "[a-z]{1,12}",
        change_provider in any::<bool>(),
    ) {
        let baseline = run(&branch_call_choice_v1());
        let mut story = branch_call_choice_v1();
        let reference = match &mut story.content[entry] {
            ContentEntryV1::Ref(reference) => reference,
            ContentEntryV1::Segment(segment) => &mut segment.unit,
        };
        let replacement = if change_provider {
            ContentRef::new(&provider, reference.key.as_str()).unwrap()
        } else {
            ContentRef::new(reference.provider.as_str(), &key).unwrap()
        };
        prop_assume!(*reference != replacement);
        *reference = replacement;
        let changed = run(&story);
        prop_assert_ne!(changed.artifact, baseline.artifact);
        for (changed, baseline) in changed.commits.iter().zip(&baseline.commits) {
            prop_assert_ne!(changed, baseline);
        }
        for (changed, baseline) in changed.states.iter().zip(&baseline.states) {
            prop_assert_ne!(changed, baseline);
        }
        for (changed, baseline) in changed.receipts.iter().zip(&baseline.receipts) {
            prop_assert_ne!(changed, baseline);
        }
    }
}

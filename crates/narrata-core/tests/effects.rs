#![allow(clippy::panic, clippy::unwrap_used)]

use narrata_core::{
    CapabilityId, CapabilityVersion, CommitId, ContentLifecycle, ContentPermission, ContentQuery,
    DeliveryPolicy, EffectPayloadDigest, EntityId, ExecutionId, HostCapabilities, HostCapability,
    InputPayloadDigest, InstructionId, ObjectId, ResolvedContent, ResumeSupport, RevisionPolicy,
    RewindPolicy, StructureOccurrence, Value, ValueSchemaV0, builtin_host_capabilities,
    content::{CheckedContentResolution, ContentResolutionError},
    derive_effect_id, effect_request_digest,
};

#[test]
fn effect_id_has_a_stable_golden_and_is_execution_scoped() {
    let capability = CapabilityId::new("host.query").unwrap();
    let version = CapabilityVersion::new(1).unwrap();
    let digest = effect_request_digest(
        &capability,
        version,
        EffectPayloadDigest::from_bytes([4; 32]),
        DeliveryPolicy::RecordedQuery,
        &RewindPolicy::ReuseRecordedResponse,
    );
    let derive = |execution| {
        derive_effect_id(
            ExecutionId::from_u128(execution),
            CommitId::from_bytes([1; 32]),
            InputPayloadDigest::from_bytes([2; 32]),
            InstructionId::from_u128(3),
            5,
            digest,
        )
    };
    assert_eq!(
        derive(7).to_string(),
        "effect:d15f44734b1c78c098a2a054eb5d66e6f203c4606504096f5aebcecc4de4712d"
    );
    assert_eq!(derive(7), derive(7));
    assert_ne!(derive(7), derive(8));
}

#[test]
fn builtin_contracts_are_unique_and_host_policy_is_authoritative() {
    let builtins = builtin_host_capabilities().unwrap();
    let query = builtins
        .get(&CapabilityId::new("host.query").unwrap())
        .unwrap();
    assert_eq!(query.version, CapabilityVersion::new(1).unwrap());
    assert_eq!(
        query.allowed_policies,
        vec![(
            DeliveryPolicy::RecordedQuery,
            RewindPolicy::ReuseRecordedResponse
        )]
    );

    let duplicate = HostCapability {
        id: CapabilityId::new("host.query").unwrap(),
        version: CapabilityVersion::new(1).unwrap(),
        request_schema: Some(ValueSchemaV0::I64),
        response_schema: Some(ValueSchemaV0::I64),
        allowed_policies: Vec::new(),
    };
    assert!(HostCapabilities::new([duplicate.clone(), duplicate]).is_err());

    let resume = HostCapabilities::new(Vec::new())
        .unwrap()
        .with_scene_resume(ResumeSupport::Seek, ResumeSupport::Restart);
    assert_eq!(resume.scene_resume().animation, ResumeSupport::Seek);
    assert_eq!(resume.scene_resume().audio, ResumeSupport::Restart);
}

#[test]
fn live_presentation_content_cannot_be_used_as_branching_value() {
    let provider = CapabilityId::new("rezics.content").unwrap();
    let occurrence = StructureOccurrence {
        structure: EntityId::from_u128(1),
        occurrence: 2,
    };
    let query = ContentQuery {
        provider: provider.clone(),
        occurrence,
        permission: ContentPermission::Entitled,
        revision: RevisionPolicy::LivePresentationOnly,
    };
    let checked = CheckedContentResolution::check(
        &query,
        ResolvedContent {
            provider: provider.clone(),
            occurrence,
            permission: ContentPermission::Entitled,
            lifecycle: ContentLifecycle::Active,
            revision: ObjectId::from_bytes([9; 32]),
            value: Value::Bool(true),
        },
    )
    .unwrap();
    assert_eq!(checked.branching_value(), None);

    let retired = CheckedContentResolution::check(
        &query,
        ResolvedContent {
            provider,
            occurrence,
            permission: ContentPermission::Entitled,
            lifecycle: ContentLifecycle::Retired,
            revision: ObjectId::from_bytes([9; 32]),
            value: Value::Bool(true),
        },
    );
    assert_eq!(retired, Err(ContentResolutionError::Retired));
}

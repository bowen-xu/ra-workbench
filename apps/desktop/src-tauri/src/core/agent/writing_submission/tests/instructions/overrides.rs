use super::*;
use research_agent_core::config::{ConfigTier, InstructionConfigPatch};

#[tokio::test]
async fn overrides_require_trust_freeze_precedence_and_restore_original_versions() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let root = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source")
        .canonicalize()
        .unwrap();
    fs::write(root.join("RULES.md"), "ABCDEFGHIJ").unwrap();
    let make_request = || {
        let mut value = request(&f, Uuid::new_v4());
        value.session_config = Some(InstructionConfigPatch {
            max_bytes: Some(8),
            fallback_filenames: Some(vec!["RULES.md".into()]),
        });
        value.turn_config = Some(InstructionConfigPatch {
            max_bytes: Some(3),
            fallback_filenames: None,
        });
        value
    };
    let rejected = make_request();
    let rejected_id = rejected.request_id;
    assert_prepare_rejection(
        &f,
        rejected_id,
        service.prepare(&f.models, rejected).await,
        "DATA_SQL_OPERATION_CONFLICT",
        "writing_instructions_unauthorized",
    );
    let metadata = fs::metadata(&root).unwrap();
    instruction_trust::update(
        &mut f.store.connection().unwrap(),
        f.manuscript,
        0,
        &InstructionRootIdentity {
            canonical_path: &root,
            device: metadata.dev(),
            inode: metadata.ino(),
        },
        true,
    )
    .unwrap();
    let input = make_request();
    let id = input.request_id;
    let prepared = service.prepare(&f.models, input).await.unwrap();
    assert_eq!(prepared.record.input.instructions[0].text, "ABC");
    let layers = &prepared.record.input.instruction_configs[0].layers;
    assert_eq!(layers[layers.len() - 2].origin.tier, ConfigTier::Session);
    assert_eq!(layers.last().unwrap().origin.tier, ConfigTier::Turn);
    let revision = layers.last().unwrap().origin.revision.clone();
    let mut changed = make_request();
    changed.turn_config.as_mut().unwrap().max_bytes = Some(0);
    let changed = service.prepare(&f.models, changed).await.unwrap();
    assert!(changed.record.input.instructions[0].text.is_empty());
    assert_ne!(
        changed.record.input.instruction_configs[0]
            .layers
            .last()
            .unwrap()
            .origin
            .revision,
        revision
    );
    let mut cleared = make_request();
    cleared.turn_config.as_mut().unwrap().fallback_filenames = Some(vec![]);
    assert!(
        service
            .prepare(&f.models, cleared)
            .await
            .unwrap()
            .record
            .input
            .instructions
            .is_empty()
    );
    // Invalid overridden input must not be concealed by the valid Turn layer.
    let mut invalid = make_request();
    invalid.session_config.as_mut().unwrap().max_bytes = Some(1_048_577);
    let invalid_id = invalid.request_id;
    assert_prepare_rejection(
        &f,
        invalid_id,
        service.prepare(&f.models, invalid).await,
        "DATA_SQL_INVALID",
        "writing_instructions_scope_invalid",
    );
    fs::write(root.join("RULES.md"), "Changed").unwrap();
    let restored = service
        .restore(
            &f.models,
            WritingSubmissionReceipt {
                conversation_id: f.conversation,
                request_id: id,
                manuscript_id: f.manuscript,
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.record, prepared.record);
}

#[test]
fn wire_overrides_cannot_forge_origin_or_permissions() {
    use crate::agent::writing_commands::ConfigOverride;
    for value in [
        serde_json::json!({"origin": "application"}),
        serde_json::json!({"permissions": "all"}),
        serde_json::json!({"maxBytes": -1}),
    ] {
        assert!(serde_json::from_value::<ConfigOverride>(value).is_err());
    }
    let value =
        serde_json::from_value::<ConfigOverride>(serde_json::json!({"maxBytes": 0})).unwrap();
    assert_eq!(value.max_bytes, Some(0));
    assert_eq!(value.fallback_filenames, None);
}

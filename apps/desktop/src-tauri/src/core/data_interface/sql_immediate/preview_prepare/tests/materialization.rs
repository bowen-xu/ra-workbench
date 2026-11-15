use super::*;
use crate::agent::context_session::{
    permissions,
    sources::{Access, Part, Resource},
};

#[test]
fn real_lowering_retains_original_queue_annotations_and_skill_while_freezing_grants() {
    let mut f = Fixture::new();
    let note = Uuid::new_v4();
    f.db.lock().unwrap().execute("insert into note_pages(id,title,created_at,updated_at) values(?1,'Synthetic reference','now','now')",[note.to_string()]).unwrap();
    let package = f
        .store
        .agent_configuration_directory()
        .join(".agents/skills/review");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("SKILL.md"),
        "---\nname: review\ndescription: Review prose\n---\nPreserve evidence before editing.",
    )
    .unwrap();
    let skill =
        crate::agent::skills::application::discover(f.store.agent_configuration_directory())
            .unwrap()
            .skills
            .remove(0);
    let mut items: Vec<UserInput> =
        serde_json::from_value(f.intent.request_json["submission"]["userInput"].clone()).unwrap();
    items.push(serde_json::from_value(json!({"type":"mention","name":"Synthetic reference","target":{"kind":"file","sourceId":format!("context:{}",json!({"kind":"note","resourceId":note})),"resourceId":note}})).unwrap());
    items.push(UserInput::Annotation {
        source: Some(format!(
            "source://writings/{}/main.tex",
            f.intent.manuscript_id
        )),
        title: "Frozen synthetic passage".into(),
        text: "Frozen manuscript".into(),
        comment: "Explain the evidence".into(),
        image: None,
        display_text: None,
    });
    items.push(UserInput::Skill {
        name: skill.name,
        identity: skill.identity,
    });
    f.replace_input(items.clone());
    let resources = vec![
        Resource::Note { resource_id: note },
        Resource::Manuscript {
            resource_id: f.intent.manuscript_id.to_string(),
            group_access: None,
            new_member_access: None,
            parts: vec![Part {
                resource: Resource::ManuscriptFile {
                    resource_id: f.intent.manuscript_id,
                    path: "main.tex".into(),
                },
                label: "main.tex".into(),
                selected: true,
                access: Access::ReadOnly,
            }],
        },
    ];
    f.intent.request_json["submission"]["resources"] = json!(resources);
    f.db.lock().unwrap().execute("insert into agent_host_shell_grants(conversation_id,host_kind,resource_id,revision) values(?1,'manuscript',?2,'synthetic-grant')",params![f.intent.conversation_id.to_string(),f.intent.manuscript_id.to_string()]).unwrap();
    let prepared = f.prepare();
    assert_ne!(prepared.record.input.user_input[1], items[1]);
    assert_eq!(prepared.record.input.user_input[2..], items[2..]);
    assert_eq!(prepared.queue.inputs()[0].user_input, items);
    assert_eq!(f.queue().inputs()[0].user_input, items);
    assert_eq!(prepared.record.input.skill_instructions.len(), 1);
    assert!(
        prepared.record.input.skill_instructions[0]
            .render()
            .contains("Preserve evidence before editing.")
    );
    assert!(
        prepared
            .record
            .input
            .skills
            .iter()
            .any(|skill| skill.explicit)
    );
    assert_eq!(
        permissions::writing_paths(&prepared.record),
        Some(std::collections::HashSet::new())
    );
    let extra = prepared
        .record
        .input
        .context
        .iter()
        .find(|source| source.resource_id == note.to_string())
        .unwrap();
    let anchor: serde_json::Value = serde_json::from_str(extra.anchor.as_deref().unwrap()).unwrap();
    assert_eq!(anchor["contextAccess"], "readOnly");
    assert_eq!(anchor["supplementalContext"], true);
    let sql = f.db.lock().unwrap();
    assert_eq!(
        sql.query_row(
            "select count(*) from agent_turn_resource_grants where turn_id=?1",
            [prepared.record.turn_id.to_string()],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        sql.query_row(
            "select grant_revision from agent_turn_shell_grants where turn_id=?1",
            [prepared.record.turn_id.to_string()],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "synthetic-grant"
    );
    drop(sql);
    assert_eq!(
        f.db.replay_queue_preview_prepare(&f.intent)
            .unwrap()
            .unwrap()
            .record,
        prepared.record
    );
}

#[test]
fn different_original_structured_queue_payload_rolls_back_preparation() {
    let mut f = Fixture::new();
    let original_intent = f.intent.clone();
    let mut items: Vec<UserInput> =
        serde_json::from_value(f.intent.request_json["submission"]["userInput"].clone()).unwrap();
    items.push(UserInput::Annotation {
        source: Some(format!(
            "source://writings/{}/main.tex",
            f.intent.manuscript_id
        )),
        title: "Changed queue evidence".into(),
        text: "A different queued annotation".into(),
        comment: String::new(),
        image: None,
        display_text: None,
    });
    f.replace_input(items);
    let mut intent = original_intent;
    intent.expected_queue_revision = f.queue().revision();
    intent.request_json["expectedRevision"] = json!(intent.expected_queue_revision);
    let before = f.queue();
    assert!(matches!(
        f.db.prepare_queue_preview(&intent, f.source_for(&intent)),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    assert_eq!(f.queue(), before);
    assert_eq!(f.counts(&f.observer()), (0, 0, 0));
}

#[test]
fn sourceless_quote_survives_queue_preparation_and_reopen() {
    let mut fixture = Fixture::new();
    let mut items: Vec<UserInput> =
        serde_json::from_value(fixture.intent.request_json["submission"]["userInput"].clone())
            .unwrap();
    items.push(UserInput::Annotation {
        source: None,
        title: "对话选段".into(),
        text: "原对话中的内容\n第二行".into(),
        comment: "请解释".into(),
        image: None,
        display_text: None,
    });
    fixture.replace_input(items.clone());
    let saved = fixture.prepare();
    assert_eq!(saved.record.input.user_input, items);
    assert_eq!(saved.queue.inputs()[0].user_input, items);
    let wire = serde_json::to_value(&items).unwrap();
    assert!(wire[1].get("source").is_none());

    let fixture = fixture.reopen();
    assert_eq!(fixture.queue().inputs()[0].user_input, items);
    let replay = fixture
        .db
        .replay_queue_preview_prepare(&fixture.intent)
        .unwrap()
        .unwrap();
    assert_eq!(replay.record.input.user_input, items);
    assert_eq!(replay.queue.inputs()[0].user_input, items);
}

#[test]
fn ordinary_compact_text_is_not_a_compact_purpose_but_an_actual_header_is() {
    let mut f = Fixture::new();
    f.intent.request_json["submission"]["text"] = json!("/compact");
    f.replace_input(vec![UserInput::Text {
        text: "/compact".into(),
        text_elements: vec![],
    }]);
    let original = f.prepare();
    assert!(
        crate::agent::writing_submission::read_compaction_revision_in(
            &f.db.lock().unwrap(),
            f.intent.conversation_id,
            f.intent.request_id
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(
        f.db.replay_queue_preview_prepare(&f.intent)
            .unwrap()
            .unwrap()
            .record,
        original.record
    );
    f.db.lock().unwrap().execute("insert into agent_compaction_preparations(conversation_id,request_id,expected_revision) values(?1,?2,0)",params![f.intent.conversation_id.to_string(),f.intent.request_id.to_string()]).unwrap();
    assert!(matches!(
        f.db.read_queue_preview_prepare_status(&f.intent),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    assert!(matches!(
        f.db.prepare_queue_preview(&f.intent, f.source()),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
}

#[test]
fn changed_source_hash_has_a_safe_specific_conflict_diagnostic_and_no_partial_claim() {
    let f = Fixture::new();
    std::fs::write(
        f.store
            .manuscript_directory()
            .join(f.intent.manuscript_id.to_string())
            .join("source/main.tex"),
        "Changed synthetic bytes",
    )
    .unwrap();
    let error =
        f.db.prepare_queue_preview(&f.intent, f.source())
            .unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Conflict(_)));
    let detail = error.diagnostic().unwrap();
    assert_eq!(detail.operation_id, f.intent.request_id);
    assert_eq!(detail.phase, "queue-prepare-context");
    assert_eq!(detail.safe_cause, "writing_context_changed");
    assert!(detail.epoch.is_some());
    assert_eq!(detail.sql_fence.as_deref(), Some("0"));
    let wire = serde_json::to_value(error).unwrap();
    assert_eq!(wire["code"], "DATA_SQL_OPERATION_CONFLICT");
    assert_eq!(wire["reason"]["safeCause"], "writing_context_changed");
    assert!(!wire.to_string().contains("Changed synthetic bytes"));
    assert!(!wire.to_string().contains("main.tex"));
    assert_eq!(f.queue().revision(), 1);
    assert_eq!(f.counts(&f.observer()), (0, 0, 0));
}

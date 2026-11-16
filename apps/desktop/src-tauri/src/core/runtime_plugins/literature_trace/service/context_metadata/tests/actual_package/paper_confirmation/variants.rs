//! Original immutable portrait-template history and human-edited field through real Core.
use super::{lifecycle::*, *};
use rusqlite::params;

fn seed_template(f: &Fixture) {
    let fields = json!([
        {"id":"model_assumptions","title":"模型假设","question":"明确定义了什么模型？","claimIds":["claim_0"]},
        {"id":"validation_basis","title":"验证依据","question":"哪些原文可支持验证？","claimIds":["claim_1"]}
    ]);
    let template =
        json!({"portraitCandidateId":"historical-portrait","portraitRevision":1,"fields":fields});
    let value = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"paper_template_v1","paperTemplate":template,
    "fields":[
        {"field":"model_assumptions","status":"filled","statements":[{"id":"template-statement","text":"Original template model Ω","epistemicRole":"source_paraphrase","evidenceItemIds":["ev-quote"],"evidenceQuotes":[{"evidenceItemId":"ev-quote","quote":"evidence Ω"}]}]},
        {"field":"validation_basis","status":"not_found","statements":[]}
    ]});
    generic_reading::seed(f, value.clone());
    let portrait = json!({"kind":"paper_portrait_v1","schemaVersion":1,"coverage":"一页",
        "claims":(["genre", "mode", "paradigm"]).iter().enumerate().map(|(index,dimension)|json!({"id":format!("claim_{index}"),"dimension":dimension,"label":"形式描述","description":"明确模型","certainty":"supported","evidence":[{"evidenceItemId":"ev-quote","quote":"evidence Ω"}]})).collect::<Vec<_>>(),
        "template":fields});
    protocol::decode_candidate_value(
        portrait.clone(),
        &std::collections::HashSet::from(["ev-quote".into()]),
    )
    .unwrap();
    let hash = crate::ai::generation_support::hash_text(&portrait.to_string());
    let db = f.store.connection().unwrap();
    // Retain valid deployed historical identities, including the portrait's own run and bundle.
    db.execute("insert into ai_runs(id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,scope_hash,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) select 'portrait-run','paper_profile_classification',status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,scope_hash,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at from ai_runs where id='legacy-run'", []).unwrap();
    db.execute("insert into ai_evidence_bundles(id,run_id,scope_hash,retrieval_config_version,validation_status,created_at) select 'portrait-bundle','portrait-run',scope_hash,retrieval_config_version,validation_status,created_at from ai_evidence_bundles where id='legacy-bundle'", []).unwrap();
    db.execute("insert into ai_evidence_items(id,bundle_id,citation_key,ordinal,anchor_id,paper_id,paper_title,attachment_id,content_sha256,page_index,page_label,quote_exact,quote_prefix,quote_suffix,context_text,relation_kind,validation_status,created_at) select 'portrait-item','portrait-bundle',citation_key,ordinal,anchor_id,paper_id,paper_title,attachment_id,content_sha256,page_index,page_label,quote_exact,quote_prefix,quote_suffix,context_text,relation_kind,validation_status,created_at from ai_evidence_items where id='legacy-item'", []).unwrap();
    db.execute("insert into ai_candidates(id,run_id,evidence_bundle_id,candidate_kind,schema_version,validation_status,original_payload_hash,created_at,updated_at) values('historical-portrait','portrait-run','portrait-bundle','paper_portrait_v1',1,'valid',?1,'old','old')", [&hash]).unwrap();
    db.execute("insert into ai_candidate_revisions(id,candidate_id,revision,payload_json,payload_hash,editor_kind,source_run_id,parent_revision,change_reason,created_at) values('portrait-revision','historical-portrait',1,?1,?2,'ai','portrait-run',null,'','old')", params![portrait.to_string(),hash]).unwrap();
    db.execute("insert into ai_overview_template_runs(run_id,paper_id,portrait_candidate_id,portrait_revision,template_json,created_at) values('legacy-run','paper','historical-portrait',1,?1,'old')", [template.to_string()]).unwrap();
    let mut edited = value;
    edited["fields"][0]["statements"][0]["text"] = json!("Human-edited template model Ω");
    let hash = crate::ai::generation_support::hash_text(&edited.to_string());
    db.execute("insert into ai_candidate_revisions(id,candidate_id,revision,payload_json,payload_hash,editor_kind,source_run_id,parent_revision,change_reason,created_at) values('human-template-revision','legacy-candidate',2,?1,?2,'human',null,1,'human edit','later')", params![edited.to_string(),hash]).unwrap();
    db.execute("update ai_candidates set current_revision=2,review_state='edited',updated_at='later' where id='legacy-candidate'", []).unwrap();
}
fn request(key: &str) -> Request {
    Request::ConfirmOverview {
        draft: AiOverviewFieldConfirmationDraft {
            candidate_id: "legacy-candidate".into(),
            candidate_revision: 2,
            field_kinds: vec!["model_assumptions".into()],
            scope_kind: "field".into(),
            project_id: None,
            idempotency_key: key.into(),
        },
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_human_edited_template_field_creates_reuses_and_restores_original_unit() {
    let mut f = Fixture::new();
    seed_template(&f);
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    let first = overview(execute(&f, request("template-first")).await.unwrap());
    let unit = &first.batch.field_confirmations[0].target_reading_unit_id;
    assert_eq!(
        first.batch.field_confirmations[0].decision_kind,
        "human_edited"
    );
    assert_eq!(body(&f, unit), ("Human-edited template model Ω".into(), 1));
    let read = || {
        f.store.connection().unwrap().query_row("select u.unit_kind,u.provenance_kind,t.portrait_candidate_id,t.field_id,a.quote_exact from structured_reading_units u join reading_unit_template_fields t on t.reading_unit_id=u.id join reading_unit_evidence_links l on l.reading_unit_id=u.id join source_anchors a on a.id=l.anchor_id where u.id=?1", [unit], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?))).unwrap()
    };
    let original = read();
    assert_eq!(
        original,
        (
            "other".into(),
            "ai_accepted_after_edit".into(),
            "historical-portrait".into(),
            "model_assumptions".into(),
            "evidence Ω".into()
        )
    );
    execute(
        &f,
        Request::UndoOverview {
            draft: AiOverviewConfirmationUndoDraft {
                batch_id: first.batch.id,
                field_kind: Some("model_assumptions".into()),
                idempotency_key: "template-reset".into(),
            },
        },
    )
    .await
    .unwrap();
    let second = overview(execute(&f, request("template-second")).await.unwrap());
    assert_eq!(
        &second.batch.field_confirmations[0].target_reading_unit_id,
        unit
    );
    assert_eq!(body(&f, unit).1, 2);
    assert_eq!(read(), original);
    execute(
        &f,
        Request::UndoOverview {
            draft: AiOverviewConfirmationUndoDraft {
                batch_id: second.batch.id,
                field_kind: None,
                idempotency_key: "template-restore".into(),
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(body(&f, unit), ("Human-edited template model Ω".into(), 3));
    assert_eq!(read(), original);
    f.service.shutdown().await.unwrap();
}

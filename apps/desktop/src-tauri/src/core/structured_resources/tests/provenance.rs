use super::*;
use rusqlite::params;

#[test]
fn snapshot_sources_mark_replaced_attachments_stale_without_changing_frozen_quote() {
    let db = database();
    paper(&db);
    let original = "a".repeat(64);
    let replacement = "b".repeat(64);
    for sha in [&original, &replacement] {
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?1,'application/pdf',1,'now')",[sha]).unwrap();
    }
    db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('attachment','paper',?1,'paper.pdf','now')",[&original]).unwrap();
    db.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,raw_response,created_at,finished_at) values('parse','paper','attachment',?1,'fixture','success','success','{}','now','now')",[&original]).unwrap();
    db.execute("insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at) values('snapshot','paper',?1,1,'now')",[&original]).unwrap();
    db.execute("insert into paper_context_items(snapshot_id,context_item_id,attachment_id,content_sha256,parse_run_id,raw_ref,item_kind,page_index,regions_json,text_content,created_at) values('snapshot','P1-T1','attachment',?1,'parse','text-1','text',0,'[]','Original frozen text.','now')",[&original]).unwrap();
    let first = crate::ai::structured_resource_validation::evidence_with(
        &crate::hbs_storage::test_sources::ReadingEvidence,
        &db,
        "paper",
        Some("snapshot"),
    )
    .unwrap();
    assert_eq!(first.items[0].current_anchor_status, "page_level");
    db.execute(
        "update attachments set content_sha256=?1 where id='attachment'",
        [&replacement],
    )
    .unwrap();
    let stale = crate::ai::structured_resource_validation::evidence_with(
        &crate::hbs_storage::test_sources::ReadingEvidence,
        &db,
        "paper",
        Some("snapshot"),
    )
    .unwrap();
    assert_eq!(stale.items[0].current_anchor_status, "stale");
    assert_eq!(stale.items[0].content_sha256, original);
    assert_eq!(stale.items[0].quote_exact, "Original frozen text.");
}

fn candidate(db: &Connection, id: &str, quote: &str, field: &str) {
    candidate_fixture(db, id, quote, field, None);
}

#[test]
fn diagnostics_preserve_exact_quote_index_and_portrait_constraint_path() {
    let db = database();
    let target = paper(&db);
    let sha = "a".repeat(64);
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'paper.pdf','application/pdf',1,'now')",[&sha]).unwrap();
    db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('attachment','paper',?1,'paper.pdf','now')",[&sha]).unwrap();
    candidate(&db, "source", "The   pur- pose.", "research_question");
    let mut value: serde_json::Value = serde_json::from_str(&reading("研究问题")).unwrap();
    value["fields"][0]["statements"][0]["evidenceQuotes"] = json!([
        {"evidenceItemId":"P1-T1","quote":"pur- pose"},
        {"evidenceItemId":"P1-T1","quote":"purpose"}
    ]);
    let errors = super::super::validation::validate_write_with(
        &mut crate::hbs_storage::test_sources::Sources::new(),
        &db,
        &target,
        &value.to_string(),
        Some("source"),
        false,
    )
    .map_err(crate::ai::structured_resource_validation::Flow::into_static)
    .unwrap_err();
    assert_eq!(
        errors[0].path,
        "$.fields[0].statements[0].evidenceQuotes[1].quote"
    );
    assert!(errors[0].message.contains("pur- pose"));
    assert!(errors[0].message.contains("purpose"));

    let mut portrait = target.clone();
    portrait.kind = "portrait".into();
    let claims = ["genre", "mode", "paradigm"].iter().map(|id| json!({"id":id,"dimension":id,"label":"不确定","description":"证据不足","certainty":"uncertain","evidence":[]})).collect::<Vec<_>>();
    let value = json!({"kind":"paper_portrait_v1","schemaVersion":1,"coverage":"已读摘录",
        "claims":claims,
        "template":[{"id":"question","title":"问题","question":"研究的问题","claimIds":["missing"]}]});
    let errors = super::super::validation::validate_write_with(
        &mut crate::hbs_storage::test_sources::Sources::rejecting_at(
            value.clone(),
            "$.template[0].claimIds[0]",
            "引用的判断 ID 不存在于 claims；请同步修复引用",
        ),
        &db,
        &portrait,
        &value.to_string(),
        Some("source"),
        false,
    )
    .map_err(crate::ai::structured_resource_validation::Flow::into_static)
    .unwrap_err();
    assert_eq!(errors[0].path, "$.template[0].claimIds[0]");
    assert!(errors[0].message.contains("不存在"));
}

#[test]
fn partial_generic_presentation_and_extra_manual_units_remain_editable() {
    for extra in [false, true] {
        let mut db = database();
        let target = paper(&db);
        let sha = "a".repeat(64);
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'paper.pdf','application/pdf',1,'now')",[&sha]).unwrap();
        db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('attachment','paper',?1,'paper.pdf','now')",[&sha]).unwrap();
        candidate(&db, "source", "Original source.", "research_question");
        if extra {
            db.execute("insert into structured_reading_units(id,paper_id,unit_kind,title,body,epistemic_role,created_at,updated_at) values('free-unit','paper','other','自由正文','不应丢弃的自由人工正文。','working_note','now','now')",[]).unwrap();
        }
        import_legacy_with_evidence(
            &crate::hbs_storage::test_sources::ReadingEvidence,
            &mut db,
            LegacyImportDraft {
                request_id: "partial-generic".into(),
                catalog: None,
                paper_id: Some("paper".into()),
                presentation: Some(LegacyPresentation {
                    fields: vec![LegacyField {
                        candidate_id: "source".into(),
                        field_id: "research_question".into(),
                    }],
                    hidden_unit_ids: vec![],
                }),
            },
        )
        .unwrap();
        let imported = get(&db, &target.id).unwrap();
        let content = imported.content.as_ref().unwrap();
        assert_eq!(content["fields"].as_array().unwrap().len(), 6);
        if extra {
            assert_eq!(content["retainedFields"][0]["field"], "other");
            assert_eq!(
                content["retainedFields"][0]["statements"][0]["text"],
                "不应丢弃的自由人工正文。"
            );
        } else {
            assert!(content.get("retainedFields").is_none());
        }
        let mut input = draft(&target.id, &content.to_string(), "edit-generic-import");
        input.expected_revision = imported.revision;
        input.editor_kind = "human".into();
        let edited = crate::hbs_storage::test_sources::write(&mut db, input).unwrap();
        assert_eq!(edited.status, "applied", "{:?}", edited.errors);
    }
}
pub(super) fn candidate_fixture(
    db: &Connection,
    id: &str,
    quote: &str,
    field: &str,
    custom: Option<serde_json::Value>,
) {
    let sha = "a".repeat(64);
    db.execute("insert into ai_runs(id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,scope_hash,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values(?1,'paper_overview','needs_review','fixture-generation','fixture-v1','v1','v1',1,?2,1000,1000,5000,?1,?1)",params![id,sha]).unwrap();
    db.execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,rects_json,quote_exact,quote_prefix,quote_suffix,created_at) values(?1,'paper','attachment',?2,0,'[]',?3,'','','now')",params![id,sha,quote]).unwrap();
    db.execute("insert into ai_evidence_bundles(id,run_id,scope_hash,retrieval_config_version,validation_status,created_at) values(?1,?1,?2,'v1','valid','now')",params![id,sha]).unwrap();
    db.execute("insert into ai_evidence_items(id,bundle_id,citation_key,ordinal,anchor_id,paper_id,paper_title,attachment_id,content_sha256,page_index,quote_exact,context_text,relation_kind,validation_status,created_at) values(?1,?1,'P1-T1',0,?1,'paper','Test','attachment',?2,0,?3,?3,'context','valid','now')",params![id,sha,quote]).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&reading("占位内容。")).unwrap();
    for entry in value["fields"].as_array_mut().unwrap() {
        *entry = if entry["field"] == field {
            json!({"field":field,"status":"filled","statements":[{"id":"statement","text":quote,"epistemicRole":"source_paraphrase","evidenceItemIds":["P1-T1"],"evidenceQuotes":[{"evidenceItemId":"P1-T1","quote":quote}]}]})
        } else {
            json!({"field":entry["field"],"status":"not_found","statements":[]})
        };
    }
    let value = custom.unwrap_or(value);
    let raw = value.to_string();
    let digest = hash(&raw);
    db.execute("insert into ai_candidates(id,run_id,evidence_bundle_id,candidate_kind,schema_version,original_payload_hash,validation_status,created_at,updated_at) values(?1,?1,?1,?3,1,?2,'valid',?1,?1)",params![id,digest,value["kind"].as_str().unwrap()]).unwrap();
    db.execute("insert into ai_candidate_revisions(id,candidate_id,revision,payload_json,payload_hash,editor_kind,source_run_id,created_at) values(?1,?1,1,?2,?3,'ai',?1,'now')",params![id,raw,digest]).unwrap();
}

#[test]
fn legacy_reading_snapshot_wins_over_absent_or_newer_portrait_and_portrait_only_stays_a_proposal() {
    for scenario in ["absent-portrait", "newer-portrait", "portrait-only"] {
        let mut db = database();
        let target = paper(&db);
        let sha = "a".repeat(64);
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'paper.pdf','application/pdf',1,'now')",[&sha]).unwrap();
        db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('attachment','paper',?1,'paper.pdf','now')",[&sha]).unwrap();
        if scenario != "portrait-only" {
            let payload = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"paper_template_v1","paperTemplate":{"portraitCandidateId":"original-portrait","portraitRevision":1,"fields":[{"id":"question","title":"旧正式标题","question":"旧字段定义","claimIds":[]}]},"fields":[{"field":"question","status":"filled","statements":[{"id":"answer","text":"Original reading source.","epistemicRole":"source_paraphrase","evidenceItemIds":["P1-T1"],"evidenceQuotes":[{"evidenceItemId":"P1-T1","quote":"Original reading source."}]}]}]});
            candidate_fixture(
                &db,
                "reading-a",
                "Original reading source.",
                "question",
                Some(payload),
            );
        }
        if scenario != "absent-portrait" {
            let portrait = json!({"kind":"paper_portrait_v1","schemaVersion":1,"coverage":"新版画像建议","claims":[{"id":"genre","dimension":"genre","label":"不确定","description":"证据不足。","certainty":"uncertain","evidence":[]},{"id":"mode","dimension":"mode","label":"不确定","description":"证据不足。","certainty":"uncertain","evidence":[]},{"id":"paradigm","dimension":"paradigm","label":"不确定","description":"证据不足。","certainty":"uncertain","evidence":[]}],"template":[{"id":"replacement","title":"新画像模板","question":"新版建议字段","claimIds":[]}]});
            candidate_fixture(
                &db,
                "portrait-z",
                "Portrait source.",
                "question",
                Some(portrait),
            );
        }
        import_legacy_with_evidence(
            &crate::hbs_storage::test_sources::ReadingEvidence,
            &mut db,
            LegacyImportDraft {
                request_id: scenario.into(),
                catalog: None,
                paper_id: Some("paper".into()),
                presentation: None,
            },
        )
        .unwrap();
        let template = ensure_paper_template(&db, "paper").unwrap();
        let reading = get(&db, &target.id).unwrap();
        if scenario == "portrait-only" {
            assert!(template.content.is_none());
            assert!(reading.content.is_none());
            assert!(reading.template_ref.is_none());
            let proposal = read_text(&db, &template.id, None, false)
                .unwrap()
                .draft
                .unwrap();
            assert!(proposal.content.contains("新画像模板"));
        } else {
            assert_eq!(
                template.content.as_ref().unwrap()["fields"][0]["title"],
                "旧正式标题",
                "{scenario}"
            );
            assert_eq!(
                reading.template_ref.as_ref().unwrap().resource_id,
                template.id
            );
            let mut input = draft(
                &target.id,
                &reading.content.as_ref().unwrap().to_string(),
                &format!("{scenario}-edit"),
            );
            input.expected_revision = reading.revision;
            input.editor_kind = "human".into();
            let result = crate::hbs_storage::test_sources::write(&mut db, input).unwrap();
            assert_eq!(result.status, "applied", "{scenario}: {:?}", result.errors);
        }
    }
}

fn adopt(db: &mut Connection, resource: &Resource, fields: Vec<&str>, request: &str) -> Resource {
    let template = super::super::legacy::imported_template(
        db,
        request,
        request,
        &json!(
            fields
                .into_iter()
                .map(|id| json!({"id":id,"title":id,"question":id}))
                .collect::<Vec<_>>()
        ),
    )
    .unwrap();
    crate::hbs_storage::test_sources::adopt(
        db,
        AdoptionDraft {
            paper_id: "paper".into(),
            template_id: template.id,
            template_revision: template.revision,
            expected_revision: resource.revision,
            confirmed: true,
            request_id: request.into(),
        },
    )
    .unwrap()
}

#[test]
fn mixed_legacy_fields_keep_exact_sources_through_edit_undo_and_template_reactivation() {
    let mut db = database();
    let target = paper(&db);
    let sha = "a".repeat(64);
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'aa/paper.pdf','application/pdf',1,'now')",[&sha]).unwrap();
    db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('attachment','paper',?1,'paper.pdf','now')",[&sha]).unwrap();
    candidate(
        &db,
        "candidate-a",
        "First exact source.",
        "research_question",
    );
    candidate(&db, "candidate-b", "Different exact source.", "approach");
    import_startup_with_evidence(
        Some(&crate::hbs_storage::test_sources::ReadingEvidence),
        &mut db,
        StartupImportDraft {
            catalog: None,
            contexts: vec![],
            presentations: vec![StartupPresentation {
                paper_id: "paper".into(),
                presentation: LegacyPresentation {
                    fields: [
                        "research_question",
                        "approach",
                        "key_result",
                        "conclusion",
                        "limitation",
                        "open_question",
                    ]
                    .into_iter()
                    .map(|field| LegacyField {
                        candidate_id: if field == "research_question" {
                            "candidate-a"
                        } else {
                            "candidate-b"
                        }
                        .into(),
                        field_id: field.into(),
                    })
                    .collect(),
                    hidden_unit_ids: vec![],
                },
            }],
        },
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let imported = get(&db, &target.id).unwrap();
    assert_ne!(
        imported.content.as_ref().unwrap()["fields"][0]["statements"][0]["id"],
        imported.content.as_ref().unwrap()["fields"][1]["statements"][0]["id"]
    );
    assert_eq!(db.query_row("select json_extract(payload_json,'$.fields[0].statements[0].id') from ai_candidate_revisions where candidate_id='candidate-a'",[],|r|r.get::<_,String>(0)).unwrap(),"statement");
    let sources = super::super::provenance::sources(
        &crate::hbs_storage::test_sources::ReadingEvidence,
        &db,
        &imported,
    )
    .unwrap();
    let question = sources
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["fieldIds"]
                .as_array()
                .unwrap()
                .contains(&json!("research_question"))
        })
        .unwrap();
    assert_eq!(question["quoteExact"], "First exact source.");
    let kept = decision(&mut db, &imported, "keep", None, None);
    let verified = decision(
        &mut db,
        &kept,
        "verify",
        None,
        Some(vec!["research_question"]),
    );
    let moved = adopt(&mut db, &verified, vec!["approach"], "move-inactive");
    assert_eq!(
        super::super::provenance::field_sources(
            &db,
            &moved.id,
            moved.revision,
            "$retained:research_question"
        )
        .unwrap(),
        vec!["candidate-a"]
    );
    assert!(
        moved
            .reviewed_field_ids
            .contains(&"$retained:research_question".into())
    );
    let restored = adopt(
        &mut db,
        &moved,
        vec!["research_question", "approach"],
        "restore-active",
    );
    assert_eq!(
        super::super::provenance::field_sources(
            &db,
            &restored.id,
            restored.revision,
            "research_question"
        )
        .unwrap(),
        vec!["candidate-a"]
    );
    assert!(
        restored
            .reviewed_field_ids
            .contains(&"research_question".into())
    );
    let mut wrong = restored.content.clone().unwrap();
    wrong["fields"][0]["statements"][0]["evidenceQuotes"][0]["quote"] =
        json!("Different exact source.");
    let mut input = draft(&restored.id, &wrong.to_string(), "cross-source-quote");
    input.expected_revision = restored.revision;
    assert_eq!(
        crate::hbs_storage::test_sources::write(&mut db, input)
            .unwrap()
            .status,
        "invalid"
    );
    let mut input = draft(
        &restored.id,
        &restored.content.as_ref().unwrap().to_string(),
        "repair-mixed",
    );
    input.expected_revision = restored.revision;
    let repaired = crate::hbs_storage::test_sources::write(&mut db, input).unwrap();
    assert_eq!(repaired.status, "applied", "{:?}", repaired.errors);
}

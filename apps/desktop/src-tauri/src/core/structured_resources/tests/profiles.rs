use super::*;

fn startup() -> StartupImportDraft {
    StartupImportDraft {
        catalog: None,
        presentations: vec![],
        contexts: vec![],
    }
}

#[test]
fn all_deployed_profiles_map_to_distinct_stable_exact_global_versions() {
    let mut db = database();
    let imported = import_startup_with_evidence(
        Some(&crate::hbs_storage::test_sources::ReadingEvidence),
        &mut db,
        startup(),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(imported.imported_template_count, 3);
    let mut ids = std::collections::HashSet::new();
    for (id, label) in [
        ("generic_overview", "通用阅读"),
        ("technical_empirical", "技术／实证"),
        ("philosophical_conceptual", "哲学／概念"),
    ] {
        let template = super::super::legacy_profiles::resolve(&db, id, 1).unwrap();
        assert!(ids.insert(template.id));
        assert_eq!(template.revision, 1);
        assert_eq!(template.content.as_ref().unwrap()["name"], label);
        assert!(
            template.content.as_ref().unwrap()["fields"]
                .as_array()
                .unwrap()
                .iter()
                .all(|field| field["question"].as_str().unwrap().ends_with('？'))
        );
    }
    let repeated = import_startup_with_evidence(
        Some(&crate::hbs_storage::test_sources::ReadingEvidence),
        &mut db,
        startup(),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(repeated.imported_template_count, 0);
    let after = [
        "generic_overview",
        "technical_empirical",
        "philosophical_conceptual",
    ]
    .map(|id| {
        super::super::legacy_profiles::resolve(&db, id, 1)
            .unwrap()
            .id
    })
    .into_iter()
    .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids, after);
}

#[test]
fn frozen_profile_composition_and_manual_body_migrate_to_an_editable_paper_template() {
    let mut db = database();
    let target = paper(&db);
    let sha = "a".repeat(64);
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'paper.pdf','application/pdf',1,'now')",[&sha]).unwrap();
    db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('attachment','paper',?1,'paper.pdf','now')",[&sha]).unwrap();
    let candidate = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"reviewed_profile_composition_v1","profileRefs":[{"profileId":"technical_empirical","profileVersion":1},{"profileId":"philosophical_conceptual","profileVersion":1}],"fields":[{"field":"research_question","status":"filled","statements":[{"id":"question","text":"Frozen profile source.","epistemicRole":"source_paraphrase","evidenceItemIds":["P1-T1"]}]},{"field":"argument","status":"not_found","statements":[]}]});
    super::provenance::candidate_fixture(
        &db,
        "frozen-profile-source",
        "Frozen profile source.",
        "research_question",
        Some(candidate),
    );
    db.execute("insert into paper_reading_profile_selections(paper_id,profile_refs_json,revision,created_at,updated_at) values('paper',?1,2,'now','now')",[json!([{"profileId":"generic_overview","profileVersion":1}]).to_string()]).unwrap();
    db.execute("insert into structured_reading_units(id,paper_id,unit_kind,title,body,epistemic_role,created_at,updated_at) values('argument-unit','paper','argument','论证','既有人工论证正文。','hypothesis','now','now')",[]).unwrap();
    import_startup_with_evidence(
        Some(&crate::hbs_storage::test_sources::ReadingEvidence),
        &mut db,
        startup(),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let saved = get(&db, &target.id).unwrap();
    let template = get_version(
        &db,
        &saved.template_ref.as_ref().unwrap().resource_id,
        saved.template_ref.as_ref().unwrap().revision,
    )
    .unwrap();
    assert_eq!(template.paper_id.as_deref(), Some("paper"));
    assert_eq!(
        template.content.as_ref().unwrap()["fields"][1]["id"],
        "argument"
    );
    let content = saved.content.as_ref().unwrap();
    assert_eq!(content["profileId"], "paper_template_v1");
    assert_eq!(
        content["fields"][0]["statements"][0]["evidenceQuotes"][0]["quote"],
        "Frozen profile source."
    );
    assert_eq!(
        content["fields"][1]["statements"][0]["text"],
        "既有人工论证正文。"
    );
    let mut input = draft(&target.id, &content.to_string(), "edit-composed-import");
    input.expected_revision = saved.revision;
    input.editor_kind = "human".into();
    let edited = crate::hbs_storage::test_sources::write(&mut db, input).unwrap();
    assert_eq!(edited.status, "applied", "{:?}", edited.errors);
}

#[test]
fn explicitly_selected_single_profile_without_body_binds_its_exact_global_version() {
    let mut db = database();
    let target = paper(&db);
    db.execute("insert into paper_reading_profile_selections(paper_id,profile_refs_json,revision,created_at,updated_at) values('paper',?1,1,'now','now')",[json!([{"profileId":"philosophical_conceptual","profileVersion":1}]).to_string()]).unwrap();
    let result = import_startup_with_evidence(
        Some(&crate::hbs_storage::test_sources::ReadingEvidence),
        &mut db,
        startup(),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(result.imported_paper_count, 1);
    let selected = get(&db, &target.id).unwrap();
    let template =
        super::super::legacy_profiles::resolve(&db, "philosophical_conceptual", 1).unwrap();
    assert!(selected.content.is_none());
    assert_eq!(selected.template_ref.unwrap().resource_id, template.id);
}

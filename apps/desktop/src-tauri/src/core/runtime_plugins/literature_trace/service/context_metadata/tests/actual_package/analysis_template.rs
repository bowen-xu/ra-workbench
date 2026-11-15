//! Original template resolution through real Core resources and the selected package.
use super::*;
use crate::{
    ai::candidate_contracts::PaperTemplateRef,
    structured_resources::{self as resources, AdoptionDraft, WriteDraft},
};

pub(in super::super) fn adopted(store: &crate::storage::AppStore) -> PaperTemplateRef {
    let mut db = store.connection().unwrap();
    let template =
        resources::create_template(&db, "Extraction template", "analysis-template-create", None)
            .unwrap();
    let template=crate::hbs_storage::test_sources::write(&mut db,WriteDraft {
        resource_id:template.id, expected_revision:template.revision,
        content:json!({"kind":"reading_template_v1","schemaVersion":1,"name":"Extraction template","fields":[{"id":"question","title":"问题","question":"研究什么？"}]}).to_string(),
        request_id:"analysis-template-write".into(), conversation_id:None, expected_draft_revision:None, source_snapshot_id:None,submission_id:None,editor_kind:"human".into(),
    }).unwrap().resource;
    let reading = resources::find_by_paper(&db, "paper", "structuredReading").unwrap();
    crate::hbs_storage::test_sources::adopt(
        &mut db,
        AdoptionDraft {
            paper_id: "paper".into(),
            template_id: template.id.clone(),
            template_revision: template.revision,
            expected_revision: reading.revision,
            confirmed: true,
            request_id: "analysis-template-adopt".into(),
        },
    )
    .unwrap();
    PaperTemplateRef::Resource {
        template_resource_id: template.id,
        template_resource_revision: template.revision,
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_template_binding_keeps_first_adoption_confirmation_reframe_and_rollback() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let template = resources::create_template(
        &f.store.connection().unwrap(),
        "Binding template",
        "bind-create",
        None,
    )
    .unwrap();
    f.bind_actual_package(catalog, key);
    let write = |resource: &resources::Resource, content: Value, request: &str| WriteDraft {
        resource_id: resource.id.clone(),
        expected_revision: resource.revision,
        content: content.to_string(),
        request_id: request.into(),
        conversation_id: None,
        expected_draft_revision: None,
        source_snapshot_id: None,
        submission_id: None,
        editor_kind: "human".into(),
    };
    let template = f
        .service
        .write_resource(
            f.store.clone(),
            write(
                &template,
                json!({
                    "kind":"reading_template_v1","schemaVersion":1,"name":"Binding template",
                    "fields":[{"id":"question","title":"问题","question":"研究什么？"}]
                }),
                "bind-template-write",
            ),
        )
        .await
        .unwrap()
        .resource;
    let reading =
        resources::find_by_paper(&f.store.connection().unwrap(), "paper", "structuredReading")
            .unwrap();
    let first = AdoptionDraft {
        paper_id: "paper".into(),
        template_id: template.id.clone(),
        template_revision: template.revision,
        expected_revision: reading.revision,
        confirmed: false,
        request_id: "bind-first".into(),
    };
    let bound = f
        .service
        .bind_resource_template(f.store.clone(), first.clone())
        .await
        .unwrap();
    assert_eq!(bound.revision, reading.revision + 1);
    assert_eq!(
        bound.template_ref.as_ref().unwrap().resource_id,
        template.id
    );
    assert_eq!(
        bound.template_ref.as_ref().unwrap().revision,
        template.revision
    );
    let replay = f
        .service
        .bind_resource_template(f.store.clone(), first)
        .await
        .unwrap();
    assert_eq!(json!(replay), json!(bound));
    let snapshot = json!({"templateResourceId":template.id,"templateResourceRevision":template.revision,
        "fields":[{"id":"question","title":"问题","question":"研究什么？","claimIds":[]}]});
    let body = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"paper_template_v1",
        "paperTemplate":snapshot,"fields":[{"field":"question","status":"not_found","statements":[]}]});
    let saved = f
        .service
        .write_resource(f.store.clone(), write(&bound, body, "bind-reading-body"))
        .await
        .unwrap();
    assert_eq!(saved.status, "applied");
    let reading = saved.resource;
    let mut content = template.content.clone().unwrap();
    content["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"method","title":"方法","question":"方法是什么？"}));
    let template = f
        .service
        .write_resource(
            f.store.clone(),
            write(&template, content, "bind-template-next"),
        )
        .await
        .unwrap()
        .resource;
    let mut next = AdoptionDraft {
        paper_id: "paper".into(),
        template_id: template.id.clone(),
        template_revision: template.revision,
        expected_revision: reading.revision,
        confirmed: false,
        request_id: "bind-next".into(),
    };
    let unconfirmed = f
        .service
        .bind_resource_template(f.store.clone(), next.clone())
        .await
        .unwrap_err();
    assert_eq!(unconfirmed.code, "invalid_input");
    assert!(
        unconfirmed
            .message
            .contains("替换已有阅读内容的模板需要用户确认")
    );
    next.confirmed = true;
    f.store.connection().unwrap().execute_batch("create temp trigger reject_template_pin before update of template_id on structured_resources begin select raise(abort,'private template pin'); end;").unwrap();
    let failed = f
        .service
        .bind_resource_template(f.store.clone(), next.clone())
        .await
        .unwrap_err();
    assert_eq!(failed.code, "storage_error");
    assert!(!failed.message.contains("private template pin"));
    assert_eq!(failed.request_id.as_deref(), Some(next.request_id.as_str()));
    assert_eq!(
        json!(resources::get(&f.store.connection().unwrap(), &reading.id).unwrap()),
        json!(reading)
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_template_pin")
        .unwrap();
    let reframed = f
        .service
        .bind_resource_template(f.store.clone(), next.clone())
        .await
        .unwrap();
    assert_eq!(reframed.revision, reading.revision + 1);
    assert_eq!(
        reframed.template_ref.as_ref().unwrap().revision,
        template.revision
    );
    let fields = reframed.content.as_ref().unwrap()["fields"]
        .as_array()
        .unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0], reading.content.as_ref().unwrap()["fields"][0]);
    assert_eq!(fields[1]["field"], "method");
    let replay = f
        .service
        .bind_resource_template(f.store.clone(), next)
        .await
        .unwrap();
    assert_eq!(json!(replay), json!(reframed));
    assert!(f.store.connection().unwrap().is_autocommit());
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_resolves_adopted_template_and_preserves_lazy_rejections() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let reference = adopted(&f.store);
    let expected = prepared_template(&f.store.connection().unwrap(), "paper", &reference).unwrap();
    f.bind_actual_package(catalog, key.clone());
    assert!(f.store.generation_policy_source().is_none());
    let result = f
        .service
        .resolve_analysis_template(f.store.clone(), &key, "paper", &reference)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&result).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
    assert_eq!(result.field_kinds, ["question"]);
    let changed = PaperTemplateRef::Resource {
        template_resource_id: reference.id().into(),
        template_resource_revision: reference.revision() + 1,
    };
    let expected =
        prepared_template(&f.store.connection().unwrap(), "paper", &changed).unwrap_err();
    let failure = f
        .service
        .resolve_analysis_template(f.store.clone(), &key, "paper", &changed)
        .await
        .unwrap_err();
    assert_eq!(failure.code(), expected.code());
    assert!(failure.to_string().contains(&expected.to_string()));
    if let crate::ai::error::AiError::NativeWorkflow { failure, .. } = failure {
        assert_eq!(failure.package_key.as_deref(), Some(key.as_str()));
        assert!(failure.instance_id.is_some());
        assert!(failure.request_id.is_some());
    } else {
        panic!("domain rejection must keep its Native request identity");
    }
    // A current resource database rejects legacy references before querying
    // any portrait candidate or attempting to decode unrelated historical JSON.
    let legacy = PaperTemplateRef::Legacy {
        portrait_candidate_id: "missing".into(),
        portrait_revision: 1,
    };
    let expected = prepared_template(&f.store.connection().unwrap(), "paper", &legacy).unwrap_err();
    let failure = f
        .service
        .resolve_analysis_template(f.store.clone(), &key, "paper", &legacy)
        .await
        .unwrap_err();
    assert_eq!(failure.code(), expected.code());
    assert!(failure.to_string().contains(&expected.to_string()));
    assert!(f.store.connection().unwrap().is_autocommit());
    f.service.shutdown().await.unwrap();
}

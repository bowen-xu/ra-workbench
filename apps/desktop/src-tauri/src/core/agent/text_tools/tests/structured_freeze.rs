use super::*;

fn frozen_fixture() -> (Fixture, String, String) {
    let mut f = fixture(true);
    let (target, template) = {
        let db = f.tools.store.connection().unwrap();
        db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('frozen-paper','document','Frozen','now','now')",[]).unwrap();
        let target =
            crate::structured_resources::find_by_paper(&db, "frozen-paper", "structuredReading")
                .unwrap();
        let template =
            crate::structured_resources::create_template(&db, "Version 2", "freeze-template", None)
                .unwrap();
        for revision in [1, 2] {
            let body = json!({"kind":"reading_template_v1","schemaVersion":1,"name":format!("Version {revision}"),"fields":[{"id":"question","title":format!("Question {revision}"),"question":"What question?"}]});
            db.execute("insert into structured_resource_revisions(resource_id,revision,content_json,baseline_json,reviewed_fields_json,protected_fields_json,editor_kind) values(?1,?2,?3,?3,'[]','[]','human')",params![template.id,revision,body.to_string()]).unwrap();
            db.execute("update structured_resources set revision=?2,content_json=?3,baseline_json=?3 where id=?1",params![template.id,revision,body.to_string()]).unwrap();
        }
        db.execute(
            "update structured_resources set template_id=?2,template_revision=1 where id=?1",
            params![target.id, template.id],
        )
        .unwrap();
        f.tools.record.input.context = resource_grants::grants(
            &db,
            &[
                Resource::StructuredObject {
                    resource_id: target.id.clone(),
                    access: Some(Access::ReadWrite),
                },
                Resource::StructuredObject {
                    resource_id: template.id.clone(),
                    access: Some(Access::ReadOnly),
                },
            ],
            Some(crate::agent::writing_resources::test_sources::source()),
        )
        .unwrap();
        for source in &mut f.tools.record.input.context {
            if source.resource_id == target.id {
                let mut anchor: Value =
                    serde_json::from_str(source.anchor.as_deref().unwrap()).unwrap();
                anchor["frozenTemplateRef"] = json!({"resourceId":template.id,"revision":1});
                source.anchor = Some(anchor.to_string());
            }
        }
        (target.source_uri, template.source_uri)
    };
    (f, target, template)
}

#[tokio::test]
async fn generation_reads_the_frozen_template_version_while_ordinary_chat_reads_current() {
    let (mut f, _, template) = frozen_fixture();
    let pinned = read(&f, &template).await;
    assert_eq!(pinned["savedRevision"], 1);
    assert_eq!(pinned["name"], "Version 1");
    assert!(pinned["text"].as_str().unwrap().contains("Question 1"));
    assert!(!pinned["text"].as_str().unwrap().contains("Question 2"));
    for source in &mut f.tools.record.input.context {
        let mut anchor: Value = serde_json::from_str(source.anchor.as_deref().unwrap()).unwrap();
        anchor.as_object_mut().unwrap().remove("frozenTemplateRef");
        source.anchor = Some(anchor.to_string());
    }
    let live = read(&f, &template).await;
    assert_eq!(live["savedRevision"], 2);
    assert_eq!(live["name"], "Version 2");
    assert!(live["text"].as_str().unwrap().contains("Question 2"));
}

#[tokio::test]
async fn generation_cannot_write_after_the_user_changes_the_adopted_template_even_after_reread() {
    let (f, target, template) = frozen_fixture();
    let id = target.rsplit('/').next().unwrap();
    let template_id = template.rsplit('/').next().unwrap();
    f.tools
        .store
        .connection()
        .unwrap()
        .execute(
            "update structured_resources set template_revision=2,revision=revision+1 where id=?1",
            [id],
        )
        .unwrap();
    read(&f, &target).await; // A fresh current-object read does not unfreeze the task.
    let body=json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"paper_template_v1","paperTemplate":{"templateResourceId":template_id,"templateResourceRevision":2,"fields":[{"id":"question","title":"Question 2","question":"What question?","claimIds":[]}]},"fields":[{"field":"question","status":"not_found","statements":[]}]}).to_string();
    let patch = format!("*** Begin Patch\n*** Update File: {target}\n@@\n+{body}\n*** End Patch");
    let result = f.tools.apply("stale-generation", &patch).await.unwrap();
    assert_eq!(result["status"], "failed", "{result}");
    assert_eq!(result["targets"][0]["status"], "failed", "{result}");
    assert!(
        result.to_string().contains("adopted template changed"),
        "{result}"
    );
    let resource =
        crate::structured_resources::get(&f.tools.store.connection().unwrap(), id).unwrap();
    assert!(resource.content.is_none());
    assert_eq!(resource.template_ref.unwrap().revision, 2);
}

use super::*;
use crate::agent::context_session::{group_scope, structured::TEMPLATE_CATALOG_ID};

#[test]
fn paper_objects_have_distinct_paths_and_inherit_effective_group_permissions_without_bodies() {
    let (_root, store, id) = fixture();
    let scope = Resource::PaperGroup {
        resource_id: "paper".into(),
        parts: vec![],
        group_access: Some(sources::Access::ReadOnly),
        new_member_access: Some(sources::Access::ReadWrite),
    };
    let tools = tools(&store, id, vec![scope.clone()]);
    let listed = tools
        .evaluate(&call(
            "research_find_sources",
            json!({"resourceId":"paper","resourceType":"portrait"}),
        ))
        .unwrap();
    assert_eq!(listed["total"], 1);
    let row = &listed["sources"][0];
    assert_eq!(row["access"], "readOnly");
    assert!(
        row["sourceUrl"]
            .as_str()
            .unwrap()
            .starts_with("source://library/paper/")
    );
    assert_ne!(row["resourceId"], "paper");
    let db = store.connection().unwrap();
    let members = group_scope::parts(
        &db,
        &scope,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let ids = members
        .iter()
        .filter_map(|part| match &part.resource {
            Resource::StructuredObject { resource_id, .. } => Some(resource_id.clone()),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), 3);
    assert!(
        members
            .iter()
            .all(|part| part.access == sources::Access::ReadOnly)
    );
    assert!(
        !serde_json::to_string(&tools.record.input.context)
            .unwrap()
            .contains("content_json")
    );
    let descriptor =
        crate::structured_resources::get_header(&db, row["resourceId"].as_str().unwrap()).unwrap();
    assert!(
        !crate::agent::context_session::structured::access(
            &db,
            &tools.record,
            &descriptor.id,
            true
        )
        .unwrap()
    );
}

#[test]
fn global_template_catalog_does_not_expose_paper_owned_templates() {
    let (_root, store, id) = fixture();
    let global = tools(
        &store,
        id,
        vec![Resource::TemplateCatalog {
            resource_id: TEMPLATE_CATALOG_ID.into(),
            access: Some(sources::Access::ReadOnly),
        }],
    );
    let listed = global
        .evaluate(&call("research_list_sources", json!({})))
        .unwrap();
    assert_eq!(listed["total"], 1);
    let db = store.connection().unwrap();
    let own = crate::structured_resources::ensure_paper_resources(&db, "paper")
        .unwrap()
        .into_iter()
        .find(|resource| resource.kind == "readingTemplate")
        .unwrap();
    assert!(
        !crate::agent::context_session::structured::access(&db, &global.record, &own.id, false)
            .unwrap()
    );
    let member = sources::Part {
        resource: Resource::StructuredObject {
            resource_id: own.id,
            access: None,
        },
        label: "Template".into(),
        selected: true,
        access: sources::Access::ReadWrite,
    };
    let owner = Resource::PaperGroup {
        resource_id: "another-paper".into(),
        parts: vec![],
        group_access: None,
        new_member_access: None,
    };
    assert_eq!(
        crate::agent::context_session::catalog::validate_parts(
            &db,
            &owner,
            "another-paper",
            &[member],
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(Error::InvalidInput)
    );
}

#[test]
fn source_directory_reads_metadata_without_parsing_saved_structured_bodies() {
    let (_root, store, id) = fixture();
    {
        let db = store.connection().unwrap();
        db.execute(
            "update structured_resources set reviewed_fields_json='{}' where paper_id='paper'",
            [],
        )
        .unwrap();
        assert!(
            crate::agent::context_session::catalog::search(
                crate::context_catalog_app::source(),
                &db,
                "Fixture",
                Some(crate::agent::writing_resources::test_sources::source())
            )
            .is_ok()
        );
    }
    let tools = tools(
        &store,
        id,
        vec![Resource::PaperPortrait {
            resource_id: "paper".into(),
        }],
    );
    let result = tools
        .evaluate(&call("research_list_sources", json!({})))
        .unwrap();
    assert_eq!(result["total"], 1);
    assert_eq!(result["sources"][0]["resourceType"], "portrait");
}

#[test]
fn structured_pdf_view_discloses_only_authorized_file_and_keeps_a_native_submission_proof() {
    let (_root, store, id) = fixture();
    {
        let db = store.connection().unwrap();
        for (file, sha, text) in [
            ("selected", "a".repeat(64), "Authorized original evidence"),
            ("unselected", "b".repeat(64), "Unshared secret original"),
        ] {
            db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?2,'application/pdf',1,'now')",params![sha,format!("{file}.pdf")]).unwrap();
            db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values(?1,'paper',?2,?3,'now')",params![file,sha,format!("{file}.pdf")]).unwrap();
            let raw = json!({
                "pages":{"1":{"page_no":1,"size":{"width":200.0,"height":400.0}},"2":{"page_no":2,"size":{"width":200.0,"height":400.0}}},
                "texts":[
                    {"self_ref":"#/texts/0","text":text,"prov":[{"page_no":1,"bbox":{"l":20.0,"t":40.0,"r":120.0,"b":80.0}}]},
                    {"self_ref":"#/texts/1","text":"Unselected page text","prov":[{"page_no":2,"bbox":{"l":20.0,"t":40.0,"r":120.0,"b":80.0}}]},
                    {"self_ref":"#/texts/2","text":"Unknown page text"}
                ]
            });
            db.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at) values(?1,'paper',?2,?3,'fixture','success','success',?4,'{}','now','now')",params![format!("parse-{file}"),file,sha,raw.to_string()]).unwrap();
        }
    }
    let tools = tools(
        &store,
        id,
        vec![Resource::Paper {
            resource_id: "paper".into(),
            attachment_id: Some("selected".into()),
            page_index: Some(0),
        }],
    );
    let result = tools
        .evaluate(&call(
            "research_read_source",
            json!({"fileId":"selected","view":"structured"}),
        ))
        .unwrap();
    assert!(result.to_string().contains("evidenceItemId"), "{result}");
    assert!(result.to_string().contains("Authorized original evidence"));
    assert!(!result.to_string().contains("Unshared secret original"));
    assert!(!result.to_string().contains("Unselected page text"));
    assert!(!result.to_string().contains("Unknown page text"));
    let db = store.connection().unwrap();
    let proof:String=db.query_row("select snapshot_json from agent_text_read_scopes where conversation_id=?1 and scope_request_id=?2 and path='source://library/paper'",params![id.to_string(),tools.record.request_id.to_string()],|row|row.get(0)).unwrap();
    let proof: Value = serde_json::from_str(&proof).unwrap();
    assert_eq!(proof["kind"], "structuredEvidenceRead");
    let count:i64=db.query_row("select count(*) from paper_context_items where snapshot_id=?1 and attachment_id='unselected'",[proof["snapshotId"].as_str().unwrap()],|row|row.get(0)).unwrap();
    assert_eq!(count, 0);
    let count:i64=db.query_row("select count(*) from paper_context_items where snapshot_id=?1 and (page_index is null or page_index!=0)",[proof["snapshotId"].as_str().unwrap()],|row|row.get(0)).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn legacy_portrait_read_only_override_is_preserved_when_object_ids_are_introduced() {
    let (_root, store, _) = fixture();
    let scope = Resource::PaperGroup {
        resource_id: "paper".into(),
        group_access: Some(sources::Access::ReadWrite),
        new_member_access: Some(sources::Access::ReadWrite),
        parts: vec![sources::Part {
            resource: Resource::PaperPortrait {
                resource_id: "paper".into(),
            },
            label: "Legacy portrait".into(),
            selected: true,
            access: sources::Access::ReadOnly,
        }],
    };
    let db = store.connection().unwrap();
    let parts = group_scope::parts(
        &db,
        &scope,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let portrait=parts.iter().find(|part|matches!(&part.resource,Resource::StructuredObject{resource_id,..} if crate::structured_resources::get_header(&db,resource_id).unwrap().kind=="portrait")).unwrap();
    assert_eq!(portrait.access, sources::Access::ReadOnly);
}

#[test]
fn exact_object_discovery_can_filter_its_paper_without_reading_siblings() {
    let (_root, store, id) = fixture();
    let object =
        crate::structured_resources::list_headers(&store.connection().unwrap(), Some("paper"))
            .unwrap()
            .into_iter()
            .find(|object| object.kind == "portrait")
            .unwrap();
    let tools = tools(
        &store,
        id,
        vec![Resource::StructuredObject {
            resource_id: object.id.clone(),
            access: Some(sources::Access::ReadOnly),
        }],
    );
    for filter in ["paper", object.id.as_str()] {
        let list = tools
            .evaluate(&call("research_list_sources", json!({"resourceId":filter})))
            .unwrap();
        assert_eq!(list["total"], 1);
        assert_eq!(list["sources"][0]["resourceId"], object.id);
        assert_eq!(list["sources"][0]["ownerResourceId"], "paper");
    }
    assert_eq!(
        tools.evaluate(&call(
            "research_list_sources",
            json!({"resourceId":"foreign"})
        )),
        Err(Error::SourceUnavailable)
    );
}

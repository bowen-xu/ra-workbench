use super::*;
use crate::agent::context_session::sources::{Access, Part};

fn literature_note(store: &AppStore) -> String {
    let mut db = store.connection().unwrap();
    let tx = db.transaction().unwrap();
    let id = crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "算法整理".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        Some("paper"),
        None,
    )
    .unwrap();
    tx.commit().unwrap();
    id
}

#[tokio::test]
async fn group_ceiling_controls_discovery_and_the_real_note_patch_tool() {
    use crate::agent::composed_tools::RejectingToolExecutor;
    let (_root, store, id) = fixture();
    let page = literature_note(&store);
    let scope = |access| Resource::PaperGroup {
        resource_id: "paper".into(),
        new_member_access: Some(Access::ReadWrite),
        group_access: Some(access),
        parts: vec![Part {
            resource: Resource::PaperNotes {
                resource_id: "paper".into(),
                excluded_note_ids: vec![],
            },
            label: "Notes".into(),
            selected: true,
            access: Access::ReadWrite,
        }],
    };
    for access in [Access::ReadOnly, Access::ReadWrite] {
        let readonly = access == Access::ReadOnly;
        let owner = tools(&store, id, vec![scope(access)]);
        let found = owner
            .evaluate(&call(
                "research_find_sources",
                json!({"resourceId":"paper","resourceType":"note","nameQuery":"算法整理"}),
            ))
            .unwrap();
        assert_eq!(
            found["sources"][0]["access"],
            if readonly { "readOnly" } else { "readWrite" }
        );
        let (_, executor) = crate::agent::text_tools::registration(
            store.clone(),
            owner.record,
            OwnedToolRegistration::new(vec![], Arc::new(RejectingToolExecutor)),
        )
        .into_parts();
        let target = format!("source://notes/{page}");
        let read = executor
            .authorize_and_execute(call("research_read_file", json!({"path":target})))
            .await
            .unwrap();
        assert!(!read.is_error, "{}", read.content);
        let patch = format!(
            "*** Begin Patch\n*** Update File: {target}\n@@\n+Algorithm details\n*** End Patch"
        );
        let result = executor
            .authorize_and_execute(call("research_apply_patch", json!({"patch":patch})))
            .await
            .unwrap();
        if readonly {
            assert!(
                result.is_error || result.content.contains("outside the selected"),
                "{}",
                result.content
            );
        } else {
            assert!(
                !result.is_error && result.content.contains("applied"),
                "{}",
                result.content
            );
        }
    }
}

#[test]
fn a_temporary_note_reference_does_not_duplicate_or_downgrade_its_group_grant() {
    let (_root, store, id) = fixture();
    let page = literature_note(&store);
    let group = Resource::PaperGroup {
        resource_id: "paper".into(),
        new_member_access: Some(Access::ReadWrite),
        group_access: Some(Access::ReadWrite),
        parts: vec![],
    };
    let target = Resource::Note {
        resource_id: Uuid::parse_str(&page).unwrap(),
    };
    let scoped = tools(&store, id, vec![group.clone(), target.clone()]);
    assert_eq!(scoped.record.input.context.len(), 1);
    assert!(
        note_access(
            &store,
            &scoped.record,
            &store.connection().unwrap(),
            &page,
            true,
            None
        )
        .unwrap()
    );
    let excluded = Resource::PaperGroup {
        resource_id: "paper".into(),
        new_member_access: Some(Access::ReadWrite),
        group_access: Some(Access::ReadWrite),
        parts: vec![Part {
            resource: Resource::PaperNotes {
                resource_id: "paper".into(),
                excluded_note_ids: vec![page.clone()],
            },
            label: "Notes".into(),
            selected: true,
            access: Access::ReadWrite,
        }],
    };
    let separate = tools(&store, id, vec![excluded, target]);
    assert_eq!(separate.record.input.context.len(), 2);
    assert!(
        note_access(
            &store,
            &separate.record,
            &store.connection().unwrap(),
            &page,
            false,
            None
        )
        .unwrap()
    );
    assert!(
        !note_access(
            &store,
            &separate.record,
            &store.connection().unwrap(),
            &page,
            true,
            None
        )
        .unwrap()
    );
}

#[test]
fn whole_paper_notes_follow_current_turn_access_and_exclusions_without_annotation_sources() {
    let (_root, store, id) = fixture();
    let page = literature_note(&store);
    let scope = |access, excluded_note_ids| Resource::PaperGroup {
        resource_id: "paper".into(),
        new_member_access: Some(Access::ReadOnly),
        group_access: None,
        parts: vec![Part {
            resource: Resource::PaperNotes {
                resource_id: "paper".into(),
                excluded_note_ids,
            },
            label: "阅读笔记".into(),
            selected: true,
            access,
        }],
    };
    let read = tools(&store, id, vec![scope(Access::ReadOnly, vec![])]);
    let write = tools(&store, id, vec![scope(Access::ReadWrite, vec![])]);
    let excluded = tools(
        &store,
        id,
        vec![scope(Access::ReadWrite, vec![page.clone()])],
    );
    {
        let db = store.connection().unwrap();
        assert!(note_access(&store, &read.record, &db, &page, false, None).unwrap());
        assert!(!note_access(&store, &read.record, &db, &page, true, None).unwrap());
        assert!(note_access(&store, &write.record, &db, &page, true, None).unwrap());
        assert!(!note_access(&store, &excluded.record, &db, &page, false, None).unwrap());
        let catalog = super::super::super::catalog::search(
            crate::context_catalog_app::source(),
            &db,
            "Fixture",
            Some(crate::agent::writing_resources::test_sources::source()),
        )
        .unwrap();
        let Resource::PaperGroup { parts, .. } = &catalog[0].resource else {
            panic!("missing paper")
        };
        assert!(parts.iter().any(|part| part.label == "算法整理"));
        assert!(
            parts
                .iter()
                .filter(|part| matches!(
                    part.resource,
                    Resource::PaperNotes { .. }
                        | Resource::PaperPortrait { .. }
                        | Resource::StructuredObject { .. }
                ))
                .all(|part| part.access == Access::ReadWrite)
        );
    }
    let later = literature_note(&store);
    let found = write
        .evaluate(&call(
            "research_find_sources",
            json!({"resourceId":"paper","resourceType":"note","nameQuery":"算法整理"}),
        ))
        .unwrap();
    assert_eq!(found["total"], 2);
    assert!(found.to_string().contains(&later));
    assert!(
        found["sources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["access"] == "readWrite")
    );
    store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set status='trashed' where id=?1",
            [&page],
        )
        .unwrap();
    let db = store.connection().unwrap();
    assert!(!note_access(&store, &write.record, &db, &page, false, None).unwrap());
}
fn pdf(store: &AppStore, id: &str) {
    let db = store.connection().unwrap();
    db.execute("insert or ignore into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values (?1,'fixture.pdf','application/pdf',1,'now')",["a".repeat(64)]).unwrap();
    db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values (?1,'paper',?2,?1,'now')",params![id,"a".repeat(64)]).unwrap();
}
#[test]
fn a_saved_item_scope_discovers_new_pdfs_reads_only_docling_and_observes_deletion() {
    let (_root, store, id) = fixture();
    let excluded = Resource::Paper {
        resource_id: "paper".into(),
        attachment_id: Some("excluded".into()),
        page_index: None,
    };
    pdf(&store, "excluded");
    let tools = tools(
        &store,
        id,
        vec![Resource::PaperGroup {
            resource_id: "paper".into(),
            new_member_access: Some(Access::ReadOnly),
            group_access: None,
            parts: vec![Part {
                resource: excluded.clone(),
                label: "Excluded".into(),
                selected: false,
                access: Access::ReadOnly,
            }],
        }],
    );
    assert_eq!(tools.record.input.context.len(), 1);
    assert_eq!(
        tools
            .evaluate(&call("research_list_sources", json!({})))
            .unwrap()["total"],
        4
    ); // metadata, portrait, structured reading, paper template
    pdf(&store, "new");
    let list = tools
        .evaluate(&call("research_list_sources", json!({})))
        .unwrap();
    assert_eq!(list["total"], 5);
    assert!(!list.to_string().contains("excluded"));
    let found = tools
        .evaluate(&call(
            "research_find_sources",
            json!({"resourceId":"paper","resourceType":"paper","nameQuery":"new"}),
        ))
        .unwrap();
    assert_eq!(found["total"], 1); // Filename discovery needs no Docling parse.
    let source = list["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["resourceType"] == "paper")
        .unwrap()["sourceId"]
        .clone();
    assert_eq!(
        tools.evaluate(&call("research_read_source", json!({"sourceId":source}))),
        Err(Error::PaperNotParsed)
    );
    let json = serde_json::json!({"texts":[{"self_ref":"#/texts/0","text":"New Docling body","prov":[{"page_no":1}]}]});
    store.connection().unwrap().execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at) values ('parse','paper','new',?1,'fixture','success','success',?2,'{}','now','now')",params!["a".repeat(64),json.to_string()]).unwrap();
    assert!(
        tools
            .evaluate(&call("research_read_source", json!({"sourceId":source})))
            .unwrap()
            .to_string()
            .contains("New Docling body")
    );
    let denied = resource_grants::grants(
        &store.connection().unwrap(),
        &[excluded],
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap()
    .remove(0)
    .source_id;
    assert_eq!(
        tools.evaluate(&call("research_read_source", json!({"sourceId":denied}))),
        Err(Error::SourceUnavailable)
    );
    store
        .connection()
        .unwrap()
        .execute("delete from attachments where id='new'", [])
        .unwrap();
    assert_eq!(
        tools
            .evaluate(&call("research_list_sources", json!({})))
            .unwrap()["total"],
        4
    );
    assert_eq!(
        tools.evaluate(&call("research_read_source", json!({"sourceId":source}))),
        Err(Error::SourceUnavailable)
    );
}
#[test]
fn selecting_one_pdf_does_not_authorize_later_attachments() {
    let (_root, store, id) = fixture();
    pdf(&store, "one");
    let tools = tools(
        &store,
        id,
        vec![Resource::Paper {
            resource_id: "paper".into(),
            attachment_id: Some("one".into()),
            page_index: None,
        }],
    );
    pdf(&store, "later");
    let list = tools
        .evaluate(&call("research_list_sources", json!({})))
        .unwrap();
    assert_eq!(list["total"], 1);
    assert!(!list.to_string().contains("later"));
}

#[test]
fn deleting_unselected_note_inventory_does_not_revoke_the_item_scope() {
    let (_root, store, id) = fixture();
    let page = note(&store, "Temporary note");
    let tools = tools(
        &store,
        id,
        vec![Resource::PaperGroup {
            resource_id: "paper".into(),
            new_member_access: Some(Access::ReadOnly),
            group_access: None,
            parts: vec![Part {
                resource: Resource::Note {
                    resource_id: Uuid::parse_str(&page).unwrap(),
                },
                label: "Inventory".into(),
                selected: false,
                access: Access::ReadOnly,
            }],
        }],
    );
    let db = store.connection().unwrap();
    db.execute("delete from reading_note_sources where page_id=?1", [&page])
        .unwrap();
    db.execute(
        "update note_pages set status='trashed' where id=?1",
        [&page],
    )
    .unwrap();
    drop(db);
    let list = tools
        .evaluate(&call("research_list_sources", json!({})))
        .unwrap();
    assert_eq!(list["total"], 4);
    assert!(!list.to_string().contains(&page));
}

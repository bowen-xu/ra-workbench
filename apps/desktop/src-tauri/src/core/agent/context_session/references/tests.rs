use super::*;
use crate::agent::context_session::{
    resource_grants,
    sources::{Access, Part, Resource},
};
use serde_json::json;
use uuid::Uuid;

#[path = "tests/qualification.rs"]
mod qualification;

fn fixture() -> (tempfile::TempDir, AppStore, Uuid, Uuid) {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values ('paper','document','Fixture','now','now'),('other','document','Other','now','now')", []).unwrap();
    let tx = db.transaction().unwrap();
    let ordinary = crate::notes::create_page_with_paper_in(
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
    let foreign = crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Other note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        Some("other"),
        None,
    )
    .unwrap();
    tx.commit().unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    (
        root,
        store,
        Uuid::parse_str(&ordinary).unwrap(),
        Uuid::parse_str(&foreign).unwrap(),
    )
}
fn mention(page: Uuid) -> Vec<UserInput> {
    vec![UserInput::Mention {
        name: "算法整理".into(),
        target: MentionTarget::File {
            source_id: format!("context:{}", json!({"kind":"note","resourceId":page})),
            resource_id: page.to_string(),
            source_url: None,
            revision: "current".into(),
        },
    }]
}
fn group(ceiling: Access, excluded: Vec<String>, dynamic: bool) -> Resource {
    Resource::PaperGroup {
        resource_id: "paper".into(),
        new_member_access: dynamic.then_some(Access::ReadWrite),
        group_access: Some(ceiling),
        parts: vec![Part {
            resource: Resource::PaperNotes {
                resource_id: "paper".into(),
                excluded_note_ids: excluded,
            },
            label: "阅读笔记".into(),
            selected: true,
            access: Access::ReadWrite,
        }],
    }
}

#[test]
fn grouped_note_reference_resolves_and_validates_without_a_duplicate_grant_or_body_snapshot() {
    let (_root, store, page, _) = fixture();
    let db = store.connection().unwrap();
    for dynamic in [false, true] {
        for ceiling in [Access::ReadOnly, Access::ReadWrite] {
            let resources = vec![
                group(ceiling.clone(), vec![], dynamic),
                Resource::Note { resource_id: page },
            ];
            let context = resource_grants::grants(&db, &resources, None).unwrap();
            assert_eq!(context.len(), 1);
            let original = context.clone();
            let mut input = mention(page);
            resolve(&db, &store, &mut input, &context).unwrap();
            assert_eq!(context, original);
            let member = resource_grants::note_reference(&db, &context, page)
                .unwrap()
                .unwrap();
            assert!(
                matches!(&input[0], UserInput::Mention {target:MentionTarget::File {source_id,resource_id,..},..}
                if source_id == &member.source_id && resource_id == &page.to_string())
            );
            let anchor: serde_json::Value =
                serde_json::from_str(member.anchor.as_deref().unwrap()).unwrap();
            assert_eq!(
                anchor["contextAccess"],
                serde_json::to_value(ceiling).unwrap()
            );
            assert_eq!(anchor["content"], "resourceGrant");
            crate::agent::skills::injection::validate_mentions(&store, &db, &input, &context)
                .unwrap();
            let restored: Vec<ContextText> =
                serde_json::from_str(&serde_json::to_string(&context).unwrap()).unwrap();
            crate::agent::skills::injection::validate_mentions(&store, &db, &input, &restored)
                .unwrap();
        }
    }
}

#[test]
fn excluded_or_foreign_notes_cannot_resolve_through_group_authority() {
    let (_root, store, page, foreign) = fixture();
    let db = store.connection().unwrap();
    for target in [page, foreign] {
        let context = resource_grants::grants(
            &db,
            &[group(Access::ReadWrite, vec![page.to_string()], true)],
            None,
        )
        .unwrap();
        let mut input = mention(target);
        let original = input.clone();
        assert_eq!(resolve(&db, &store, &mut input, &context), Ok(()));
        assert_eq!(input, original);
    }
}

#[test]
fn unresolved_tags_do_not_block_submission_or_create_grants() {
    let (_root, store, page, foreign) = fixture();
    let db = store.connection().unwrap();
    let context =
        resource_grants::grants(&db, &[group(Access::ReadWrite, vec![], true)], None).unwrap();
    let mut input = mention(page);
    resolve(&db, &store, &mut input, &context).unwrap();
    for field in ["sourceId", "resourceId", "revision"] {
        let mut forged = serde_json::to_value(&input).unwrap();
        forged[0]["target"][field] = json!(if field == "resourceId" {
            foreign.to_string()
        } else {
            "forged".into()
        });
        let forged: Vec<UserInput> = serde_json::from_value(forged).unwrap();
        assert!(
            crate::agent::skills::injection::validate_mentions(&store, &db, &forged, &context)
                .is_ok()
        );
    }
    assert!(
        resource_grants::note_reference(&db, &context, foreign)
            .unwrap()
            .is_none()
    );
    let revoked = resource_grants::grants(
        &db,
        &[group(Access::ReadWrite, vec![page.to_string()], true)],
        None,
    )
    .unwrap();
    assert!(
        crate::agent::skills::injection::validate_mentions(&store, &db, &input, &revoked).is_ok()
    );
    assert!(
        resource_grants::note_reference(&db, &revoked, page)
            .unwrap()
            .is_none()
    );
}

#[test]
fn standalone_note_reference_keeps_its_existing_binding() {
    let (_root, store, page, _) = fixture();
    let db = store.connection().unwrap();
    let context =
        resource_grants::grants(&db, &[Resource::Note { resource_id: page }], None).unwrap();
    let mut input = mention(page);
    resolve(&db, &store, &mut input, &context).unwrap();
    assert!(
        matches!(&input[0], UserInput::Mention {target:MentionTarget::File {source_id,..},..} if source_id == &context[0].source_id)
    );
    crate::agent::skills::injection::validate_mentions(&store, &db, &input, &context).unwrap();
}

#[test]
fn display_address_cannot_replace_the_frozen_resource_identity() {
    let (_root, store, page, _) = fixture();
    let db = store.connection().unwrap();
    let context =
        resource_grants::grants(&db, &[Resource::Note { resource_id: page }], None).unwrap();
    let mut input = mention(page);
    if let UserInput::Mention {
        target: MentionTarget::File { source_url, .. },
        ..
    } = &mut input[0]
    {
        *source_url = Some("source://library/unrelated/file".into());
    }
    resolve(&db, &store, &mut input, &context).unwrap();
    assert_eq!(
        serde_json::to_value(&input[0]).unwrap()["target"]["sourceUrl"],
        format!("source://notes/{page}")
    );
}

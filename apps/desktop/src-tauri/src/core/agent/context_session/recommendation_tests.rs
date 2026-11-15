use super::*;
use rusqlite::params;
use uuid::Uuid;

#[test]
fn exact_recommendations_need_only_identity_metadata_and_preserve_membership() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table bibliographic_items(id text primary key,title text,trashed_at text);
      create table attachments(id text primary key,item_id text,display_name text,attachment_kind text,created_at text);
      create table manuscripts(id text primary key,title text,status text);
      create table manuscript_files(manuscript_id text,relative_path text,file_kind text);").unwrap();
    let project = Uuid::new_v4();
    for index in 0..70 {
        let id = if index == 0 {
            project.to_string()
        } else {
            Uuid::new_v4().to_string()
        };
        db.execute(
            "insert into manuscripts values(?1,'Same title','active')",
            [&id],
        )
        .unwrap();
        db.execute(
            "insert into manuscript_files values(?1,'main.tex','tex')",
            [&id],
        )
        .unwrap();
        db.execute(
            "insert into bibliographic_items values(?1,'Same title',null)",
            [&id],
        )
        .unwrap();
        db.execute(
            "insert into attachments values(?1,?1,'Same PDF','pdf','now')",
            [&id],
        )
        .unwrap();
    }
    for resource in [
        Resource::ManuscriptFile {
            resource_id: project,
            path: "main.tex".into(),
        },
        Resource::Manuscript {
            resource_id: project.to_string(),
            parts: vec![],
            group_access: None,
            new_member_access: None,
        },
        Resource::Paper {
            resource_id: project.to_string(),
            attachment_id: Some(project.to_string()),
            page_index: None,
        },
    ] {
        let result = resolve(
            &db,
            Selection {
                label: "Stale title that does not match".into(),
                resource,
            },
            Some(crate::agent::writing_resources::test_sources::source()),
        )
        .unwrap();
        assert_eq!(result.label, "Same title");
        match result.resource {
            Resource::Manuscript {
                resource_id,
                parts,
                new_member_access,
                group_access,
            }
            | Resource::PaperGroup {
                resource_id,
                parts,
                new_member_access,
                group_access,
            } => {
                assert_eq!(resource_id, project.to_string());
                assert!(parts.is_empty());
                assert_eq!(new_member_access, Some(Access::ReadWrite));
                assert_eq!(group_access, Some(Access::ReadWrite));
            }
            _ => panic!("expected a dynamic group"),
        }
    }
    let wrong = Resource::Paper {
        resource_id: project.to_string(),
        attachment_id: Some(Uuid::new_v4().to_string()),
        page_index: None,
    };
    assert!(
        resolve(
            &db,
            Selection {
                label: "wrong".into(),
                resource: wrong
            },
            Some(crate::agent::writing_resources::test_sources::source())
        )
        .is_err()
    );
    let wrong = Resource::ManuscriptFile {
        resource_id: project,
        path: "../main.tex".into(),
    };
    assert!(
        resolve(
            &db,
            Selection {
                label: "wrong".into(),
                resource: wrong
            },
            Some(crate::agent::writing_resources::test_sources::source())
        )
        .is_err()
    );
    db.execute(
        "update manuscripts set status='archived' where id=?1",
        params![project.to_string()],
    )
    .unwrap();
    assert!(
        resolve(
            &db,
            Selection {
                label: "removed".into(),
                resource: Resource::ManuscriptFile {
                    resource_id: project,
                    path: "main.tex".into()
                }
            },
            Some(crate::agent::writing_resources::test_sources::source())
        )
        .is_err()
    );
}

#[test]
fn open_project_recommendation_needs_no_file_tab_or_file_inventory() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "create table manuscripts(id text primary key,title text,status text);
        insert into manuscripts values('project','Project title','active');",
    )
    .unwrap();
    let candidate = || Selection {
        label: "Stale title".into(),
        resource: Resource::Manuscript {
            resource_id: "project".into(),
            parts: vec![],
            group_access: None,
            new_member_access: None,
        },
    };
    let result = resolve(
        &db,
        candidate(),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(result.label, "Project title");
    assert!(
        matches!(result.resource,Resource::Manuscript{resource_id,parts,group_access:Some(Access::ReadWrite),..}
        if resource_id=="project" && parts.is_empty())
    );
    db.execute("update manuscripts set status='archived'", [])
        .unwrap();
    assert!(
        resolve(
            &db,
            candidate(),
            Some(crate::agent::writing_resources::test_sources::source())
        )
        .is_err()
    );
}

#[test]
fn focused_groups_match_catalog_defaults_without_reading_bodies() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','document','Synthetic','now','now')",[]).unwrap();
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'synthetic.pdf','application/pdf',0,'now')",["a".repeat(64)]).unwrap();
    db.execute("insert into attachments(id,item_id,display_name,content_sha256,created_at) values('pdf','paper','PDF',?1,'now')",["a".repeat(64)]).unwrap();
    let recommended = resolve(
        &db,
        Selection {
            label: "old".into(),
            resource: Resource::Paper {
                resource_id: "paper".into(),
                attachment_id: Some("pdf".into()),
                page_index: None,
            },
        },
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let catalog = super::super::catalog::search(
        crate::context_catalog_app::source(),
        &db,
        "Synthetic",
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap()
    .remove(0);
    assert_eq!(
        serde_json::to_value(super::super::group_scope::compact(&recommended.resource)).unwrap(),
        serde_json::to_value(super::super::group_scope::compact(&catalog.resource)).unwrap()
    );
}

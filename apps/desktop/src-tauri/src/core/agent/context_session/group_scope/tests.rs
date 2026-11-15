use super::*;

fn db() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values ('paper','document','Fixture','now','now'),('other','document','Other','now','now')",[]).unwrap();
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values (?1,'fixture.pdf','application/pdf',1,'now')",["a".repeat(64)]).unwrap();
    db
}

#[test]
fn group_read_only_ceiling_blocks_explicit_and_future_project_writes_without_erasing_preferences() {
    let id = Uuid::new_v4();
    let scope = Resource::Manuscript {
        resource_id: id.to_string(),
        new_member_access: Some(Access::ReadWrite),
        group_access: Some(Access::ReadOnly),
        parts: vec![member(
            Resource::ManuscriptFile {
                resource_id: id,
                path: "main.tex".into(),
            },
            "Main".into(),
            &Access::ReadWrite,
        )],
    };
    assert!(!path_access(
        &scope,
        "main.tex",
        true,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
    assert!(!path_access(
        &scope,
        "future.tex",
        true,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
    assert!(path_access(
        &scope,
        "main.tex",
        false,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
    let mut restored = compact(&scope);
    if let Resource::Manuscript { group_access, .. } = &mut restored {
        *group_access = Some(Access::ReadWrite);
    }
    assert!(path_access(
        &restored,
        "main.tex",
        true,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
    assert!(path_access(
        &restored,
        "future.tex",
        true,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
}
fn attachment(db: &Connection, id: &str, owner: &str) {
    db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values (?1,?2,?3,?1,'now')",rusqlite::params![id,owner,"a".repeat(64)]).unwrap();
}
#[test]
fn attachments_are_live_with_stable_exclusions_and_foreign_ownership_checks() {
    let db = db();
    attachment(&db, "excluded", "paper");
    let scope = Resource::PaperGroup {
        resource_id: "paper".into(),
        new_member_access: Some(Access::ReadOnly),
        group_access: None,
        parts: vec![Part {
            resource: Resource::Paper {
                resource_id: "paper".into(),
                attachment_id: Some("excluded".into()),
                page_index: None,
            },
            label: "Excluded".into(),
            selected: false,
            access: Access::ReadOnly,
        }],
    };
    let grant = super::super::resource_grants::grants(
        &db,
        std::slice::from_ref(&scope),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(grant.len(), 1); // Durable scope, not an expanded file list.
    assert!(
        super::super::resource_grants::resource(&grant[0])
            .is_some_and(|resource| is_dynamic(&resource))
    );
    attachment(&db, "new", "paper");
    attachment(&db, "foreign", "other");
    let current = parts(
        &db,
        &scope,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert!(current.iter().any(|part| part.selected
        && matches!(&part.resource,Resource::Paper {attachment_id:Some(id),..} if id=="new")));
    assert!(!current.iter().any(|part|part.selected && matches!(&part.resource,Resource::Paper {attachment_id:Some(id),..} if id=="excluded" || id=="foreign")));
    db.execute("delete from attachments where id='excluded'", [])
        .unwrap();
    assert!(
        parts(
            &db,
            &scope,
            Some(crate::agent::writing_resources::test_sources::source())
        )
        .is_ok()
    );
    attachment(&db, "excluded", "paper");
    assert!(!parts(&db,&scope, Some(crate::agent::writing_resources::test_sources::source())).unwrap().iter().find(|part|matches!(&part.resource,Resource::Paper {attachment_id:Some(id),..} if id=="excluded")).unwrap().selected);
    db.execute(
        "update attachments set item_id='other' where id='excluded'",
        [],
    )
    .unwrap();
    assert!(matches!(
        parts(
            &db,
            &scope,
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(Error::InvalidInput)
    ));
}
#[test]
fn project_new_members_inherit_while_legacy_and_path_overrides_remain_narrow() {
    let db = db();
    let id = Uuid::new_v4();
    db.execute("insert into research_projects(id,title,summary,created_at) values ('project','Fixture','','now')",[]).unwrap();
    db.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values (?1,'project','Fixture','main.tex','now','now')",[id.to_string()]).unwrap();
    let file = |path: &str| {
        db.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values (?1,?2,'tex',1,?3,'now')",rusqlite::params![id.to_string(),path,"b".repeat(64)]).unwrap()
    };
    file("main.tex");
    let old = Resource::Manuscript {
        resource_id: id.to_string(),
        new_member_access: None,
        group_access: None,
        parts: vec![member(
            Resource::ManuscriptFile {
                resource_id: id,
                path: "main.tex".into(),
            },
            "Main".into(),
            &Access::ReadOnly,
        )],
    };
    let mut scope = old.clone();
    if let Resource::Manuscript {
        new_member_access,
        parts,
        ..
    } = &mut scope
    {
        *new_member_access = Some(Access::ReadWrite);
        parts.push(Part {
            resource: Resource::ManuscriptFile {
                resource_id: id,
                path: "excluded.tex".into(),
            },
            label: "Excluded".into(),
            selected: false,
            access: Access::ReadWrite,
        });
    }
    file("new.tex");
    file("excluded.tex");
    assert_eq!(
        parts(
            &db,
            &old,
            Some(crate::agent::writing_resources::test_sources::source())
        )
        .unwrap()
        .len(),
        1
    );
    let current = parts(
        &db,
        &scope,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(current.len(), 3);
    assert!(
        current.iter().any(|part| part.label == "new.tex"
            && part.selected
            && part.access == Access::ReadWrite)
    );
    assert!(path_access(
        &scope,
        "sections/future.tex",
        true,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
    assert!(path_access(
        &scope,
        "main.tex",
        false,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
    for path in [
        "main.tex",
        "excluded.tex",
        "../outside.tex",
        "/outside.tex",
        "private.key",
        "image.pdf",
    ] {
        assert!(
            !path_access(
                &scope,
                path,
                true,
                Some(crate::agent::writing_resources::test_sources::source())
            ),
            "{path}"
        );
    }
    if let Resource::Manuscript {
        new_member_access, ..
    } = &mut scope
    {
        *new_member_access = Some(Access::ReadOnly);
    }
    assert!(!path_access(
        &scope,
        "future.tex",
        true,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
    assert!(path_access(
        &scope,
        "future.tex",
        false,
        Some(crate::agent::writing_resources::test_sources::source())
    ));
}

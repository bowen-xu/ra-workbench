use super::{
    Error, catalog, search_catalog,
    sources::{Access, Resource},
};
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
    params,
};
use uuid::Uuid;

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into research_projects(id,title,summary,created_at) values('catalog-project','Catalog fixture','','now')",[]).unwrap();
    db
}
fn manuscript(db: &Connection, id: &str, title: &str, updated: &str, status: &str) {
    db.execute("insert into manuscripts(id,project_id,title,status,created_at,updated_at) values(?1,'catalog-project',?2,?3,'now',?4)",params![id,title,status,updated]).unwrap();
}
fn file(db: &Connection, id: &str, path: &str, kind: &str) {
    db.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,?2,?3,0,?4,'now')",params![id,path,kind,"a".repeat(64)]).unwrap();
}

#[tokio::test]
async fn search_catalog_uses_domain_queries_with_original_bounds_order_and_parts() {
    let db = database();
    for index in 0..=60 {
        let time = format!("{index:03}");
        db.execute("insert into bibliographic_items(id,item_type,title,abstract_text,archived_at,created_at,updated_at) values(?1,'document',?2,'',?3,'now',?4)",params![format!("paper-{index:02}"),format!("Needle paper {index:02}"),if index==60{Some("archived")}else{None},time]).unwrap();
        db.execute(
            "insert into note_pages(id,title,created_at,updated_at) values(?1,?2,'now',?3)",
            params![
                Uuid::from_u128(0x2000 + index).to_string(),
                format!("Needle note {index:02}"),
                time
            ],
        )
        .unwrap();
        manuscript(
            &db,
            &Uuid::from_u128(0x3000 + index).to_string(),
            &format!("Needle manuscript {index:02}"),
            &time,
            "active",
        );
    }
    db.execute("insert into bibliographic_items(id,item_type,title,abstract_text,created_at,updated_at) values('abstract-only','document','Background','Needle','now','102'),('attachment-only','document','Background attachment','','now','101')",[]).unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,abstract_text,trashed_at,created_at,updated_at) values('trashed','document','Needle trashed','','trashed','now','zzz')",[]).unwrap();
    db.execute("insert into note_pages(id,title,status,created_at,updated_at) values(?1,'Needle archived','archived','now','zzz')",[Uuid::from_u128(0x2999).to_string()]).unwrap();
    let path_match = Uuid::from_u128(0x3999).to_string();
    manuscript(&db, &path_match, "Path only", "100", "active");
    manuscript(
        &db,
        &Uuid::from_u128(0x3998).to_string(),
        "Needle archived",
        "zzz",
        "archived",
    );
    for (path, kind) in [
        ("Needle-image.png", "image"),
        ("z.sty", "style"),
        ("a.tex", "tex"),
        ("B.bib", "bib"),
        ("ignore.pdf", "other"),
    ] {
        file(&db, &path_match, path, kind);
    }
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'object','application/pdf',0,'now')",["b".repeat(64)]).unwrap();
    for (id, paper, name, kind, created) in [
        ("pdf-b", "paper-60", "B.pdf", "pdf", "a"),
        ("pdf-a", "paper-60", "A.pdf", "pdf", "a"),
        ("pdf-later", "paper-60", "Later.pdf", "pdf", "b"),
        ("image", "paper-60", "Image.png", "image", "0"),
        (
            "attachment-name",
            "attachment-only",
            "Needle reference.png",
            "image",
            "a",
        ),
    ] {
        db.execute("insert into attachments(id,item_id,display_name,attachment_kind,content_sha256,created_at) values(?1,?2,?3,?4,?5,?6)",params![id,paper,name,kind,"b".repeat(64),created]).unwrap();
    }
    let changes = db.total_changes();
    let temporary = tempfile::tempdir().unwrap();
    let store = crate::storage::AppStore::for_test(db, temporary.path().join("objects"));
    let rows = search_catalog(&store, "Needle".into()).await.unwrap();
    assert_eq!(rows.len(), 180); // Each original branch has its own 60-row ceiling.
    let papers = rows[..60]
        .iter()
        .map(|row| match &row.resource {
            Resource::PaperGroup { resource_id, .. } => resource_id.clone(),
            _ => panic!("paper branch order"),
        })
        .collect::<Vec<_>>();
    let mut expected = vec!["abstract-only".to_owned(), "attachment-only".to_owned()];
    expected.extend((3..=60).rev().map(|i| format!("paper-{i:02}")));
    assert_eq!(papers, expected);
    let Resource::PaperGroup { parts, .. } = &rows[1].resource else {
        panic!("attachment-name match")
    };
    assert!(
        !parts
            .iter()
            .any(|part| matches!(part.resource, Resource::Paper { .. }))
    ); // Non-PDF names can match, but never become PDF children.
    let Resource::PaperGroup {
        parts,
        group_access,
        new_member_access,
        ..
    } = &rows[2].resource
    else {
        panic!("archived paper still visible")
    };
    assert_eq!(group_access, &Some(Access::ReadWrite));
    assert_eq!(new_member_access, &Some(Access::ReadWrite));
    assert!(matches!(parts[0].resource, Resource::PaperMetadata { .. }));
    let pdfs = parts
        .iter()
        .filter_map(|part| match &part.resource {
            Resource::Paper {
                attachment_id: Some(id),
                page_index: None,
                ..
            } => {
                assert!(part.selected);
                assert_eq!(part.access, Access::ReadOnly);
                Some((id.as_str(), part.label.as_str()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        pdfs,
        [
            ("pdf-a", "A.pdf"),
            ("pdf-b", "B.pdf"),
            ("pdf-later", "Later.pdf")
        ]
    );
    let note_ids = rows[60..120]
        .iter()
        .map(|row| match &row.resource {
            Resource::NoteGroup { resource_id, .. } => resource_id.clone(),
            _ => panic!("note branch order"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        note_ids,
        (1..=60)
            .rev()
            .map(|i| Uuid::from_u128(0x2000 + i).to_string())
            .collect::<Vec<_>>()
    );
    let manuscripts = rows[120..]
        .iter()
        .map(|row| match &row.resource {
            Resource::Manuscript { resource_id, .. } => resource_id.clone(),
            _ => panic!("manuscript branch order"),
        })
        .collect::<Vec<_>>();
    let mut expected = vec![path_match];
    expected.extend(
        (2..=60)
            .rev()
            .map(|i| Uuid::from_u128(0x3000 + i).to_string()),
    );
    assert_eq!(manuscripts, expected);
    let Resource::Manuscript { parts, .. } = &rows[120].resource else {
        panic!("registered path match")
    };
    assert_eq!(
        parts
            .iter()
            .map(|part| match &part.resource {
                Resource::ManuscriptFile { path, .. } => path.as_str(),
                _ => panic!("text part"),
            })
            .collect::<Vec<_>>(),
        ["B.bib", "a.tex", "z.sty"]
    );
    assert!(
        search_catalog(&store, " Needle ".into())
            .await
            .unwrap()
            .is_empty()
    ); // Query bytes are not trimmed.
    assert_eq!(store.connection().unwrap().total_changes(), changes);
    assert!(store.connection().unwrap().is_autocommit());
}

#[test]
fn catalog_candidate_errors_and_optional_paths_keep_original_validation_order() {
    let db = database();
    let source = crate::context_catalog_app::source();
    assert!(
        catalog::search(source, &db, "not-present", None)
            .unwrap()
            .is_empty()
    );
    manuscript(&db, "not-a-uuid", "Boundary", "now", "active");
    assert_eq!(
        catalog::search(source, &db, "Boundary", None)
            .err()
            .unwrap(),
        Error::Unavailable
    );
    let rows = catalog::search(
        source,
        &db,
        "Boundary",
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert!(
        matches!(&rows[0].resource,Resource::Manuscript{resource_id,parts,..} if resource_id=="not-a-uuid"&&parts.is_empty())
    );
    file(&db, "not-a-uuid", "main.tex", "tex");
    assert_eq!(
        catalog::search(
            source,
            &db,
            "Boundary",
            Some(crate::agent::writing_resources::test_sources::source())
        )
        .err()
        .unwrap(),
        Error::Unavailable
    );
    let changes = db.total_changes();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Read {
            table_name: "bibliographic_items",
            ..
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let exact = "é".repeat(256);
    assert_eq!(exact.len(), 512);
    assert_eq!(
        catalog::search(source, &db, &exact, None).err().unwrap(),
        Error::Unavailable
    ); // The original SQL failure category is retained.
    assert_eq!(
        catalog::search(source, &db, &format!("{exact}a"), None)
            .err()
            .unwrap(),
        Error::InvalidInput
    ); // Byte budget precedes the first denied query.
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(db.total_changes(), changes);
    assert!(db.is_autocommit());
}

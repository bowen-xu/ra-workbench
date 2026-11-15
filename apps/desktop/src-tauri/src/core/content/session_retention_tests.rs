use rusqlite::Connection;
use tempfile::TempDir;

use super::*;
use crate::notes::session_assets::{HbsRead, load_document_for_session, read_hbs_for_session};

fn open_store(root: &TempDir) -> AppStore {
    let objects = root.path().join("objects");
    fs::create_dir_all(&objects).unwrap();
    let mut db = Connection::open(root.path().join("metadata.sqlite3")).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute(
        "update content_gc_state set backfill_status='complete' where id=1",
        [],
    )
    .unwrap();
    AppStore::for_test(db, objects)
}

fn insert_object(store: &AppStore, bytes: &[u8]) -> String {
    let hash = format!("{:x}", Sha256::digest(bytes));
    let relative = expected_relative_path(&hash).unwrap();
    let path = store.content_directory().join(&relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    store
        .connection()
        .unwrap()
        .execute(
            "insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)
         values(?1,?2,'image/png',?3,'now')",
            params![hash, relative.to_string_lossy(), bytes.len() as i64],
        )
        .unwrap();
    hash
}

fn create_image_page(store: &AppStore, hash: &str) {
    let db = store.connection().unwrap();
    db.execute("insert into note_pages(id,title,position,status,is_favorite,revision,schema_version,created_at,updated_at)
        values('page','Page',0,'active',0,0,1,'now','now')", []).unwrap();
    let content = serde_json::json!({"kind":"asset","assetId":hash,"displayName":"image.png","mimeType":"image/png","caption":""});
    db.execute("insert into note_blocks(id,scope_kind,page_id,position,block_type,props_json,content_json,plain_text,schema_version,revision,status,is_favorite,created_at,updated_at)
        values('image','page','page',0,'image','{}',?1,'',1,0,'active',0,'now','now')", [content.to_string()]).unwrap();
    db.execute(
        "insert into note_block_assets(block_id,content_sha256,display_name,mime_type,created_at)
        values('image',?1,'image.png','image/png','now')",
        [hash],
    )
    .unwrap();
}

fn remove_durable_roots(store: &AppStore) {
    let mut db = store.connection().unwrap();
    db.execute_batch(
        "delete from note_blocks; delete from hbs_note_baselines; delete from hbs_preservations;",
    )
    .unwrap();
    reconcile_candidates(&mut db).unwrap();
    db.execute(
        "update content_object_gc_candidates set not_before='2000-01-01T00:00:00Z'",
        [],
    )
    .unwrap();
}

#[test]
fn session_retention_is_shared_across_store_clones_and_blocks_force_gc_without_db_refs() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let hash = insert_object(&store, b"undo-only asset");
    let reader = store.clone();
    {
        let _content_read = reader.content_read().unwrap();
        reader.retain_content_for_session(&hash).unwrap();
        reader.retain_content_for_session(&hash).unwrap();
    }
    assert!(store.is_content_session_retained(&hash).unwrap());
    assert!(!has_references(&store.connection().unwrap(), &hash).unwrap());
    assert_eq!(
        sweep_candidates(&store, &ContentGcManager::default(), true)
            .unwrap()
            .0,
        0
    );
    assert!(
        store
            .content_directory()
            .join(expected_relative_path(&hash).unwrap())
            .exists()
    );
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from content_objects where sha256=?1",
                [&hash],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn session_retention_does_not_survive_reopening_the_durable_store() {
    let root = tempfile::tempdir().unwrap();
    let hash;
    {
        let store = open_store(&root);
        hash = insert_object(&store, b"old process undo asset");
        let _content_read = store.content_read().unwrap();
        store.retain_content_for_session(&hash).unwrap();
    }
    let reopened = open_store(&root);
    assert!(!reopened.is_content_session_retained(&hash).unwrap());
    assert_eq!(
        sweep_candidates(&reopened, &ContentGcManager::default(), true)
            .unwrap()
            .0,
        1
    );
    assert!(
        !reopened
            .content_directory()
            .join(expected_relative_path(&hash).unwrap())
            .exists()
    );
}

#[test]
fn session_retention_captures_loaded_document_before_any_image_byte_read() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let hash = insert_object(&store, b"never rendered image");
    create_image_page(&store, &hash);
    let snapshot = load_document_for_session(&store, "page").unwrap();
    assert_eq!(snapshot.document.blocks[0].content["assetId"], hash);
    remove_durable_roots(&store);
    assert!(!has_references(&store.connection().unwrap(), &hash).unwrap());
    assert!(store.is_content_session_retained(&hash).unwrap());
    assert_eq!(
        sweep_candidates(&store, &ContentGcManager::default(), false)
            .unwrap()
            .0,
        0
    );
    assert_eq!(
        sweep_candidates(&store, &ContentGcManager::default(), true)
            .unwrap()
            .0,
        0
    );
}

#[test]
fn session_retention_captures_hbs_document_saved_and_baseline_assets() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let saved_hash = insert_object(&store, b"saved image");
    let baseline_hash = insert_object(&store, b"baseline image");
    create_image_page(&store, &saved_hash);
    {
        let db = store.connection().unwrap();
        let mut baseline = crate::notes::load_document_value(&db, "page").unwrap();
        baseline.blocks[0].content["assetId"] = baseline_hash.clone().into();
        crate::notes::hbs::store_baseline(&db, "page", &baseline).unwrap();
    }
    let snapshot = read_hbs_for_session(&store, "note", "page", HbsRead::Document).unwrap();
    assert_eq!(
        snapshot["saved"]["blocks"][0]["content"]["assetId"],
        saved_hash
    );
    assert_eq!(
        snapshot["baseline"]["blocks"][0]["content"]["assetId"],
        baseline_hash
    );
    remove_durable_roots(&store);
    for hash in [&saved_hash, &baseline_hash] {
        assert!(!has_references(&store.connection().unwrap(), hash).unwrap());
        assert!(store.is_content_session_retained(hash).unwrap());
    }
    assert_eq!(
        sweep_candidates(&store, &ContentGcManager::default(), true)
            .unwrap()
            .0,
        0
    );
}

#[test]
fn session_retention_captures_hbs_preserved_content_and_ancestors() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let saved_hash = insert_object(&store, b"observed saved image");
    let candidate_hash = insert_object(&store, b"preserved candidate image");
    let ancestor_hash = insert_object(&store, b"preserved ancestor image");
    create_image_page(&store, &saved_hash);
    {
        let mut db = store.connection().unwrap();
        let observed = crate::hbs_storage::read(&db, "note", "page").unwrap();
        let mut content = observed["saved"].clone();
        content["blocks"][0]["content"]["assetId"] = candidate_hash.clone().into();
        let mut ancestor = observed["saved"].clone();
        ancestor["blocks"][0]["content"]["assetId"] = ancestor_hash.clone().into();
        let checkpoint = serde_json::json!({
            "format":1,"kind":"note","observed":observed,"content":content,
            "ancestor":ancestor,"baselineAncestor":observed["baseline"],"ordinary":[],
        });
        crate::hbs_storage::test_sources::preservation_save(&mut db, "note", "page", checkpoint)
            .unwrap();
    }
    let checkpoint = read_hbs_for_session(&store, "note", "page", HbsRead::Preservation).unwrap();
    assert_eq!(
        checkpoint["content"]["blocks"][0]["content"]["assetId"],
        candidate_hash
    );
    remove_durable_roots(&store);
    for hash in [&saved_hash, &candidate_hash, &ancestor_hash] {
        assert!(!has_references(&store.connection().unwrap(), hash).unwrap());
        assert!(store.is_content_session_retained(hash).unwrap());
    }
    assert_eq!(
        sweep_candidates(&store, &ContentGcManager::default(), true)
            .unwrap()
            .0,
        0
    );
}

#[test]
fn session_retention_does_not_starve_unretained_objects_beyond_a_full_candidate_batch() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    {
        let _content_read = store.content_read().unwrap();
        for index in 0..=SWEEP_OBJECT_LIMIT {
            let hash = insert_object(&store, format!("undo resource {index}").as_bytes());
            store.retain_content_for_session(&hash).unwrap();
        }
    }
    store
        .connection()
        .unwrap()
        .execute(
            "update content_object_gc_candidates set not_before='2000-01-01T00:00:00Z'",
            [],
        )
        .unwrap();
    let unretained = insert_object(&store, b"later orphan still reclaimable");
    assert_eq!(
        sweep_candidates(&store, &ContentGcManager::default(), true)
            .unwrap()
            .0,
        1
    );
    assert!(
        !store
            .content_directory()
            .join(expected_relative_path(&unretained).unwrap())
            .exists()
    );
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row("select count(*) from content_objects", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        (SWEEP_OBJECT_LIMIT + 1) as i64
    );
}

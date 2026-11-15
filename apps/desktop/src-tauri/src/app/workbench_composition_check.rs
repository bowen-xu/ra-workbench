//! Isolated development acceptance over the production Core storage/HBS owners.
use crate::{
    core_sources,
    hbs_storage::{self, Commit},
    notes,
    runtime_events::RuntimeEvents,
    storage::{AppStore, AppStorePaths, AppStorePorts},
};
use serde_json::json;
use std::path::Path;

fn open(directory: &Path) -> Result<AppStore, crate::storage::StorageError> {
    AppStore::open_with_paths(
        AppStorePaths {
            data_directory: directory.to_owned(),
            agent_configuration_directory: directory.join("configuration"),
        },
        AppStorePorts::with_events(RuntimeEvents::new(|_, _| Ok(())), core_sources::sources()),
    )
}

fn finish_core_startup(store: &AppStore) -> Result<(), Box<dyn std::error::Error>> {
    crate::structured_resources::import_startup(
        store.paper_reading_source(),
        &mut *store.connection()?,
        crate::structured_resources::StartupImportDraft {
            catalog: None,
            presentations: vec![],
            contexts: vec![],
        },
        store.writing_resource_source(),
    )?;
    Ok(())
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let allowed = root.join(".tmp");
    std::fs::create_dir_all(&allowed)?;
    if allowed.canonicalize()? != allowed {
        return Err("composition acceptance requires the repository .tmp directory".into());
    }
    let parent = allowed.join("plugin-architecture/runtime");
    std::fs::create_dir_all(&parent)?;
    let parent = parent.canonicalize()?;
    if !parent.starts_with(&allowed) {
        return Err("composition acceptance data directory escapes repository .tmp".into());
    }
    let directory = parent.join(format!("composition-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory)?;
    let store = open(&directory)?;
    finish_core_startup(&store)?;

    let page = uuid::Uuid::new_v4().to_string();
    let operation = uuid::Uuid::new_v4().to_string();
    let document = serde_json::from_value(json!({
        "schemaVersion":1,"pageId":page,"blocks":[{
            "id":"core-composition-block","type":"paragraph","props":{},
            "content":{"kind":"inline","items":[{"type":"text","text":"Core initial text","styles":[]}]},
            "children":[]
        }]
    }))?;
    let expected;
    let migration_count;
    {
        let mut db = store.connection()?;
        let initial = notes::hbs::seed(&mut db, &page, document)?;
        let mut saved = initial["saved"].clone();
        saved["blocks"][0]["content"]["items"][0]["text"] = json!("Core persisted candidate");
        let result = hbs_storage::commit(
            store.generation_policy_source(),
            store.paper_reading_source(),
            &mut db,
            "note",
            &page,
            Commit {
                id: operation.clone(),
                expected_version: initial["version"].as_i64().ok_or("missing note version")?,
                saved: saved.clone(),
                baseline: initial["baseline"].clone(),
            },
        )?;
        expected = result["snapshot"].clone();
        if expected["saved"] != saved || expected["baseline"] != initial["baseline"] {
            return Err("Core HBS save did not preserve the independent review baseline".into());
        }
        migration_count = db.query_row("select count(*) from schema_migrations", [], |row| {
            row.get::<_, usize>(0)
        })?;
        if migration_count != crate::storage_migrations::MIGRATIONS.len() {
            return Err("Core store did not apply the complete shared migration history".into());
        }
    }
    let database = store.shared_agent_connection();
    database.shutdown()?;
    drop(database);
    drop(store);

    let reopened = open(&directory)?;
    finish_core_startup(&reopened)?;
    {
        let db = reopened.connection()?;
        if notes::hbs::read(&db, &page)? != expected {
            return Err("Core note/HBS state changed after production store reopen".into());
        }
        let receipts: usize = db.query_row(
            "select count(*) from hbs_commit_receipts where kind='note' and resource_id=?1 and request_id=?2",
            rusqlite::params![page, operation],
            |row| row.get(0),
        )?;
        if receipts != 1 {
            return Err("Core note save lost or duplicated its durable receipt".into());
        }
    }

    reopened.shared_agent_connection().shutdown()?;
    let receipt = json!({
        "selectedPlugins":Vec::<String>::new(),
        "productionStoreFreshOpen":true,
        "productionStructuredStartup":true,
        "productionCoreNoteCommit":true,
        "independentHbsBaselinePreserved":true,
        "productionStoreReopen":true,
        "durableHbsReceiptCount":1,
        "migrationCount":migration_count,
        "expectedMigrationCount":crate::storage_migrations::MIGRATIONS.len(),
        "syntheticDataDirectory":directory,
        "pageId":page,
        "operationId":operation
    });

    std::fs::write(
        directory.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "ok":true,
            "selectedPlugins":Vec::<String>::new(),
            "receiptPath":directory.join("receipt.json")
        }))?
    );
    Ok(())
}

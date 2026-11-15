//! A single fixed synthetic Core store for the original structured-reading GUI.
//! Preparation is not an HBS/UI acceptance result and accepts no write payload.
use super::*;
use crate::{
    runtime_plugins::literature_reading_units::{ReadingUnitRequest, ReadingUnitResult},
    structured_resources::{self, StartupImportDraft, StartupImportResult},
};
use research_workbench_native_sdk::{
    catalog_save::CatalogDraft,
    reading_units::{StructuredReadingUnit, StructuredReadingUnitDraft},
};
use std::io::Write;

const FLAG: &str = "--structured-hbs-fixture";
const TITLE: &str = "Native HBS GUI fixture";
const UNIT_TITLE: &str = "HBS fixture question";
const BODY: &str = "Synthetic HBS pending reading text.";
const UNIT_KIND: &str = "research_question";

pub(super) fn package_argument(arguments: &[String]) -> Result<Option<&str>, Error> {
    if let [package, mode] = arguments
        && mode == FLAG
    {
        return Ok(Some(package));
    }
    if arguments
        .iter()
        .any(|argument| argument == FLAG || argument.starts_with("--structured-hbs-fixture="))
    {
        return Err("usage: literature-catalog-check <controlled-package-directory> --structured-hbs-fixture".into());
    }
    Ok(None)
}
fn check_features() -> Result<(), Error> {
    if !cfg!(debug_assertions) || !cfg!(feature = "development-data") {
        return Err("structured HBS fixture requires debug --no-default-features --features development-data".into());
    }
    Ok(())
}

pub(super) fn run(package_directory: &str) -> Result<(), Error> {
    check_features()?;
    // Discover through the real controlled package owner before allocating data.
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(package_directory.into())));
    let listing = catalog.discover()?;
    if listing.packages.len() != 1 || listing.packages[0].manifest["id"] != "literature_library" {
        return Err(
            "structured HBS fixture requires exactly one independent literature package".into(),
        );
    }
    let key = listing.packages[0].package_key.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let parent = root.join(".tmp/runtime-plugins");
    std::fs::create_dir_all(&parent)?;
    if !parent.canonicalize()?.starts_with(root.join(".tmp")) {
        return Err("structured HBS fixture requires the repository .tmp directory".into());
    }
    let directory = tempfile::Builder::new()
        .prefix("l07-hbs-fixture-")
        .tempdir_in(&parent)?
        .keep()
        .canonicalize()?;
    if !directory.starts_with(parent.canonicalize()?) {
        return Err(
            "structured HBS fixture directory escaped the repository temporary owner".into(),
        );
    }
    let store_path = directory.join("store");
    let store = AppStore::open_with_paths(
        AppStorePaths {
            data_directory: store_path.clone(),
            agent_configuration_directory: directory.join("configuration"),
        },
        AppStorePorts::with_events(RuntimeEvents::new(|_, _| Ok(())), core_sources::sources()),
    )?;
    let service = Arc::new(LiteratureTraceService::new(catalog));
    let result = runtime.block_on(async {
        let prepared = prepare(&store, &service, &key).await;
        let stopped = service.shutdown().await;
        match (prepared, stopped) {
            (Ok(receipt), Ok(())) => Ok(receipt),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error.into()),
            (Err(error), Err(shutdown)) => Err(format!(
                "fixture preparation failed: {error}; service shutdown failed: {shutdown}"
            )
            .into()),
        }
    });
    // Do this on both preparation success and failure; a failed close cannot
    // publish a prepared marker. No live service/store/runtime reaches the GUI.
    let service_released = Arc::strong_count(&service) == 1;
    let database = store.shared_agent_connection();
    let database_shutdown = database.shutdown();
    drop(database);
    drop(store);
    drop(service);
    drop(runtime);
    database_shutdown?;
    if !service_released {
        return Err("structured HBS fixture still has an operation owner after shutdown".into());
    }
    let mut receipt = result?;
    receipt["status"] = json!("prepared-structured-hbs-gui-fixture");
    receipt["packageKey"] = json!(key);
    receipt["store"] = json!(store_path);
    receipt["shutdown"] =
        json!({"service":"passed","database":"passed","serviceStoreRuntimeDropped":true});
    receipt["scope"] = json!(
        "Fixed synthetic preparation through original catalog, manual reading-unit and startup owners only; no HBS commit, Keep, preservation write, model, PDF or GUI acceptance."
    );
    let path = directory.join("fixture.json");
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    output.write_all(&serde_json::to_vec_pretty(&receipt)?)?;
    output.sync_all()?;
    println!(
        "{}",
        json!({"receipt":path,"status":receipt["status"],"store":store_path})
    );
    Ok(())
}

async fn prepare(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let draft: CatalogDraft =
        serde_json::from_value(json!({"id":null,"itemType":"journalArticle","title":TITLE,
        "issuedYear":2026,"rawBibtex":null,"sourceKind":null,"creators":[],"tags":[]}))?;
    let paper = service
        .save_catalog(store.shared_agent_connection(), key.into(), draft)
        .await?;
    uuid::Uuid::parse_str(&paper.id)?;
    if paper.item_type != "journalArticle"
        || paper.title != TITLE
        || paper.issued_year != Some(2026)
        || paper.trashed_at.is_some()
        || !paper.doi.is_empty()
        || !paper.creators.is_empty()
        || !paper.tags.is_empty()
    {
        return Err("fixed catalog readback differs from the original save result".into());
    }
    let unit = match service
        .reading_units(
            store.shared_agent_connection(),
            key.into(),
            ReadingUnitRequest::Save {
                draft: Box::new(StructuredReadingUnitDraft {
                    id: None,
                    project_id: None,
                    paper_id: Some(paper.id.clone()),
                    unit_kind: UNIT_KIND.into(),
                    title: UNIT_TITLE.into(),
                    body: BODY.into(),
                    epistemic_role: "working_note".into(),
                    anchor_id: None,
                    evidence_stance: None,
                    evidence_rationale: None,
                    expected_revision: None,
                    change_reason: Some("structured_hbs_gui_fixture".into()),
                }),
            },
        )
        .await?
    {
        ReadingUnitResult::Unit(unit) => unit,
        ReadingUnitResult::Units(_) => {
            return Err("manual reading-unit save returned a list".into());
        }
    };
    uuid::Uuid::parse_str(&unit.id)?;
    if unit.paper_id.as_deref() != Some(paper.id.as_str())
        || unit.project_id.is_some()
        || unit.template_field.is_some()
        || unit.unit_kind != UNIT_KIND
        || unit.title != UNIT_TITLE
        || unit.body != BODY
        || unit.provenance_kind != "human_original"
        || unit.epistemic_role != "working_note"
        || unit.revision != 1
        || unit.evidence_count != 0
    {
        return Err(
            "fixed manual reading-unit readback differs from original save semantics".into(),
        );
    }
    let startup = service
        .structured_startup(
            store.shared_agent_connection(),
            Some(key.into()),
            StartupImportDraft {
                catalog: None,
                presentations: vec![],
                contexts: vec![],
            },
            None,
        )
        .await?;
    readback(store, &paper, &unit, &startup)
}

fn readback(
    store: &AppStore,
    paper: &CatalogItem,
    unit: &StructuredReadingUnit,
    startup: &StartupImportResult,
) -> Result<Value, Error> {
    if startup.paper_ids != [paper.id.clone()]
        || startup.imported_paper_count != 1
        || !startup.deferred_paper_ids.is_empty()
        || !startup.deferred_context_ids.is_empty()
    {
        return Err("original startup did not finish the fixed paper import".into());
    }
    let db = store.connection()?;
    let resources = structured_resources::list(&db, Some(&paper.id))?;
    let readings = resources
        .iter()
        .filter(|resource| resource.kind == "structuredReading")
        .collect::<Vec<_>>();
    let [resource] = readings.as_slice() else {
        return Err("original startup did not produce exactly one reading resource".into());
    };
    uuid::Uuid::parse_str(&resource.id)?;
    if resource.paper_id.as_deref() != Some(paper.id.as_str())
        || resource.revision < 1
        || resource.review_baseline.is_some()
        || !resource.reviewed_field_ids.is_empty()
        || !resource
            .pending_field_ids
            .iter()
            .any(|field| field == UNIT_KIND)
    {
        return Err("original reading resource is not an unreviewed pending candidate".into());
    }
    let content = resource
        .content
        .as_ref()
        .ok_or("original reading content is absent")?;
    let fields = content["fields"]
        .as_array()
        .ok_or("original active fields are absent")?;
    let active = fields
        .iter()
        .filter(|field| field["field"] == UNIT_KIND)
        .collect::<Vec<_>>();
    let [field] = active.as_slice() else {
        return Err("research question is not an exact active field".into());
    };
    let statements = field["statements"]
        .as_array()
        .ok_or("active field has no statements")?;
    let [statement] = statements.as_slice() else {
        return Err("active field does not contain the one original unit".into());
    };
    let expected_id = format!("legacy-{}", unit.id);
    if field["status"] != "filled"
        || statement["id"] != expected_id
        || statement["text"] != BODY
        || statement["epistemicRole"] != "working_note"
        || statement["evidenceItemIds"] != json!([])
    {
        return Err("original legacy statement lost its actual unit identity or fixed body".into());
    }
    let retained = content["retainedFields"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|field| field["statements"].as_array().into_iter().flatten())
        .any(|statement| statement["id"] == expected_id || statement["text"] == BODY);
    if retained {
        return Err("the fixed active candidate was incorrectly retained".into());
    }
    let preservation_count: i64 = db.query_row(
        "select count(*) from hbs_preservations where kind='structured' and resource_id=?1",
        [&resource.id],
        |row| row.get(0),
    )?;
    let commit_count: i64 = db.query_row(
        "select count(*) from hbs_commit_receipts where kind='structured' and resource_id=?1",
        [&resource.id],
        |row| row.get(0),
    )?;
    if preservation_count != 0 || commit_count != 0 {
        return Err("preparation unexpectedly performed an HBS preservation or commit".into());
    }
    Ok(
        json!({"kind":"structured-hbs-gui-fixture","version":1,"paperId":paper.id,"paperTitle":paper.title,
        "unitId":unit.id,"unitKind":unit.unit_kind,"unitRevision":unit.revision,"unitBody":unit.body,
        "resourceId":resource.id,"resourceRevision":resource.revision,"statementId":expected_id,
        "startup":startup,"resource":resource,"checks":{"actualCoreIds":true,"manualHumanOriginal":true,
            "activeResearchQuestion":true,"exactStatement":true,"notRetained":true,"baselineNull":true,
            "unreviewed":true,"pending":true,"noPriorHbsPreservation":true,"noPriorHbsCommit":true}}),
    )
}

#[cfg(test)]
#[path = "structured_hbs_fixture/tests.rs"]
mod tests;

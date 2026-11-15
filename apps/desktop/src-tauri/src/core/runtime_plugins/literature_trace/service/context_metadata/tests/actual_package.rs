use super::*;

pub(super) fn prepared_context(
    db: &rusqlite::Connection,
    paper: &str,
    attachments: &[String],
) -> Result<crate::ai::paper_reading_contracts::PreparedPaperContext, crate::ai::error::AiError> {
    use crate::runtime_plugins::literature_context::PaperContextResources;
    let (catalog, _) = actual_package();
    let package = catalog.discover().unwrap().packages.remove(0);
    let resources = PaperContextResources::load(&catalog, &package)
        .map_err(|error| crate::ai::error::AiError::InvalidInput(error.to_string()))?;
    let captured = resources
        .capture_build(db, paper, attachments)
        .map_err(|error| {
            crate::ai::error::AiError::InvalidInput(error.failure.error.to_string())
        })?;
    Ok(research_literature_library_native::paper_context::build(&captured.input).into())
}
use crate::{agent::source_tools, runtime_plugins::RuntimePluginCatalog};
use research_agent_core::submission::{ContextText, SubmissionRecord};
use std::{fs, path::PathBuf};
mod analysis_attempt;
mod analysis_candidate;
mod analysis_completion;
mod analysis_creation;
mod analysis_docling;
#[cfg(all(debug_assertions, feature = "development-data"))]
mod analysis_docling_runner;
mod analysis_docling_storage;
mod analysis_history;
pub(super) mod analysis_overview;
mod analysis_preparation;
mod analysis_prompt;
#[cfg(all(debug_assertions, feature = "development-data"))]
mod analysis_start;
#[cfg(all(debug_assertions, feature = "development-data"))]
mod analysis_submission;
pub(super) mod analysis_template;
mod analysis_workflow;
mod catalog_save_faults;
mod concept_capture;
mod concept_evolution;
mod concept_identity;
#[cfg(all(debug_assertions, feature = "development-data"))]
mod derived_views;
mod evidence_inventory;
mod evidence_matrix;
mod evidence_write;
mod generic_reading;
mod manual_definitions;
mod manual_relations;
#[cfg(all(debug_assertions, feature = "development-data"))]
mod matrix_synthesis;
mod mixed;
mod paper_confirmation;
pub(super) mod paper_preparation;
mod paper_preview;
#[cfg(all(debug_assertions, feature = "development-data"))]
mod paper_proven;
mod paper_turn_sources;
mod portrait_admission;
mod reference_generation;
mod research_export;
#[cfg(all(debug_assertions, feature = "development-data"))]
mod retrieval_preview;
#[cfg(all(debug_assertions, feature = "development-data"))]
mod semantic_index;
mod shared_foundations;
#[cfg(all(debug_assertions, feature = "development-data"))]
mod suggestion_generation;
mod suggestion_items;
#[cfg(all(debug_assertions, feature = "development-data"))]
mod task_generation_host;

const ROOT_ENV: &str = "RESEARCH_ASSISTANT_CONTEXT_METADATA_PACKAGE_ROOT";
const KEY_ENV: &str = "RESEARCH_ASSISTANT_CONTEXT_METADATA_PACKAGE_KEY";

/// Explicit acceptance of an already built controlled artifact. Default tests do
/// not launch it, and this test neither builds nor copies a plugin package.
#[tokio::test]
#[ignore = "requires explicit repository .tmp package root and exact content key"]
async fn actual_controlled_package_metadata_bytes_match_app_static_oracle() {
    let (catalog, key) = actual_package();
    let mut fixture = Fixture::new();
    {
        let db = fixture.store.connection().unwrap();
        db.execute("update bibliographic_items set abstract_text=?1,volume='',language='zh',issued_year=2026 where id='paper'",["Unicode Ω\nquoted \"value\" and \\ slash"]).unwrap();
        db.execute("insert into identifiers(id,item_id,kind,display_value,normalized_value) values('doi','paper','doi','10.0000/Ω','10.0000/ω')",[]).unwrap();
    }
    fixture.mixed(true);
    fixture.bind_actual_package(catalog, key.clone());
    assert!(fixture.store.paper_metadata_source().is_none());
    assert!(fixture.store.paper_reading_source().is_none());
    let metadata_body = expected_metadata(&fixture.store.connection().unwrap(), "paper");
    let mut frozen = metadata_record(&fixture.record, metadata_body.clone());
    let mut request = fixture.metadata_read();
    request.arguments["startChar"] = json!(0);
    request.arguments["maxChars"] = json!(8000);
    let expected = source_tools::evaluate(&frozen, &request).unwrap();
    let full = fixture.execute(request.clone()).await;
    assert!(!full.is_error, "{}", full.content);
    let full: Value = serde_json::from_str(&full.content).unwrap();
    assert_eq!(
        full["text"].as_str().unwrap().as_bytes(),
        metadata_body.as_bytes()
    );
    compare_read(&full, &expected);
    assert_eq!(fixture.proofs(), 0, "metadata does not add a durable proof");

    request.arguments["startChar"] = json!(3);
    request.arguments["maxChars"] = json!(17);
    let expected = source_tools::evaluate(&frozen, &request).unwrap();
    let page = fixture.execute(request).await;
    assert!(!page.is_error, "{}", page.content);
    let page: Value = serde_json::from_str(&page.content).unwrap();
    compare_read(&page, &expected);
    assert_eq!(page["readViewId"], full["readViewId"]);
    assert_eq!(fixture.proofs(), 0);

    let search = fixture.search();
    let expected_metadata_search = source_tools::evaluate(&frozen, &search).unwrap();
    let first = fixture.execute(search.clone()).await;
    assert!(!first.is_error, "{}", first.content);
    let first: Value = serde_json::from_str(&first.content).unwrap();
    let metadata_id = &frozen.input.context[0].source_id;
    let actual_metadata: Vec<_> = first["matches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| value["sourceId"] == *metadata_id)
        .collect();
    let expected_matches = expected_metadata_search["matches"].as_array().unwrap();
    assert_eq!(actual_metadata.len(), expected_matches.len());
    for (actual, expected) in actual_metadata.iter().zip(expected_matches) {
        for field in [
            "sourceId",
            "kind",
            "resourceId",
            "startChar",
            "endChar",
            "snippetStartChar",
            "snippet",
        ] {
            assert_eq!(
                actual[field], expected[field],
                "metadata search field {field}"
            );
        }
    }
    assert!(first.to_string().contains("Note Ω"));
    assert!(first.to_string().contains("Portrait Ω"));
    let note_source = &fixture.record.input.context[0].source_id;
    assert_eq!(first["matches"][0]["sourceId"], *note_source);
    assert_eq!(
        fixture.proofs(),
        2,
        "only original Note and portrait read proofs exist"
    );
    let again = fixture.execute(search).await;
    assert!(!again.is_error, "{}", again.content);
    assert_eq!(
        serde_json::from_str::<Value>(&again.content).unwrap(),
        first
    );
    assert_eq!(fixture.proofs(), 2);
    {
        let host = fixture.service.host.lock().await;
        let host = host.as_ref().expect("actual package host was used");
        assert_eq!(host.package_key(), key);
        assert!(host.request_identity().is_some());
    }
    // The immutable oracle retains the same original source identity/body and
    // uses the App's existing source-tool hash/offset implementation throughout.
    frozen.input.context[0].text = expected_metadata(&fixture.store.connection().unwrap(), "paper");
    assert_eq!(
        frozen.input.context[0].text.as_bytes(),
        metadata_body.as_bytes()
    );
    fixture.service.shutdown().await.unwrap();
    assert!(fixture.service.host.lock().await.is_none());
    println!(
        "{}",
        json!({"acceptance":"actual-controlled-context-metadata-package",
        "packageKey":key,"readViewId":full["readViewId"],"metadataProofs":0,
        "mixedCoreProofs":2,"normalHostShutdown":true})
    );
}

fn metadata_record(record: &SubmissionRecord, body: String) -> SubmissionRecord {
    let mut frozen = record.clone();
    let mut source: ContextText = record
        .input
        .context
        .iter()
        .find(|source| {
            matches!(
                crate::agent::context_session::resource_grants::resource(source),
                Some(crate::agent::context_session::sources::Resource::PaperMetadata { .. })
            )
        })
        .unwrap()
        .clone();
    source.text = body;
    frozen.input.context = vec![source];
    frozen
}
fn compare_read(actual: &Value, expected: &Value) {
    for field in [
        "text",
        "sourceId",
        "turnId",
        "readViewId",
        "contentSliceHash",
        "startChar",
        "endChar",
        "totalChars",
        "nextChar",
    ] {
        assert_eq!(
            actual[field], expected[field],
            "Core source-tool field {field}"
        );
    }
}
fn actual_package() -> (Arc<RuntimePluginCatalog>, String) {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let temporary = repository.join(".tmp").canonicalize().unwrap();
    let selected = PathBuf::from(std::env::var_os(ROOT_ENV).expect(ROOT_ENV));
    assert!(
        fs::symlink_metadata(&selected)
            .unwrap()
            .file_type()
            .is_dir(),
        "package root must be a real directory"
    );
    let root = selected.canonicalize().unwrap();
    assert!(
        root.starts_with(&temporary) && root != temporary,
        "only an explicit repository .tmp descendant is accepted"
    );
    let key = std::env::var(KEY_ENV).expect(KEY_ENV);
    assert!(
        key.len() == 64
            && key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(root)).with_native_writing_preview(true));
    let listing = catalog.discover().unwrap();
    let package = listing
        .packages
        .iter()
        .find(|package| package.package_key == key)
        .expect("exact requested package must be present");
    assert_eq!(package.manifest["id"], "literature_library");
    let entry = package.manifest["native"]["entry"].as_str().unwrap();
    let bytes = catalog.resource(&key, entry).unwrap().bytes;
    assert!(
        !bytes.starts_with(b"#!"),
        "the explicit acceptance requires a compiled host, not the shell fixture"
    );
    crate::runtime_plugins::native::package::accept_context_metadata(&catalog, &key).unwrap();
    (catalog, key)
}

async fn reopen_actual_fixture(mut f: Fixture, fenced: bool) -> Fixture {
    f.service.shutdown().await.unwrap();
    let shutdown = f.store.shared_agent_connection().shutdown();
    if fenced {
        assert!(
            shutdown
                .unwrap_err()
                .to_string()
                .contains("data_recovery_required")
        );
    } else {
        shutdown.unwrap();
    }
    drop(f.store);
    f.store = crate::storage::AppStore::open_with_paths(
        crate::storage::AppStorePaths {
            data_directory: f._data.path().into(),
            agent_configuration_directory: f._data.path().join("configuration"),
        },
        crate::storage::AppStorePorts::with_events(
            crate::runtime_events::RuntimeEvents::new(|_, _| Ok(())),
            crate::storage::AppStoreSources::default(),
        ),
    )
    .unwrap();
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    f
}

#[cfg(all(debug_assertions, feature = "development-data"))]
mod paper_items;

pub(super) fn prepared_candidate(
    db: &rusqlite::Connection,
    run: &str,
    bundle: &crate::ai::candidate_contracts::AiEvidenceBundle,
    content: &str,
) -> Result<crate::ai::generation_policy::PreparedCandidate, crate::ai::error::AiError> {
    use crate::ai::{
        error::AiError,
        generation_policy::{CandidateValidationFacts, PreparedCandidate},
    };
    use research_literature_library_native::analysis_workflow::candidate;
    let keys = bundle
        .items
        .iter()
        .map(|item| item.citation_key.clone())
        .collect();
    let payload = candidate::decode_candidate_content(content, &keys).map_err(AiError::from)?;
    let facts = CandidateValidationFacts::new(db);
    let (task, suggestion) = facts.run_output(run)?;
    candidate::validate_run_candidate_kind(&task, suggestion.as_deref(), payload.kind())
        .map_err(AiError::from)?;
    research_literature_library_native::structured_validation::quotes(
        &payload,
        &candidate_quotes(bundle),
    )
    .map_err(AiError::from)?;
    candidate::validate_overview_profile(&payload, facts.overview_profile(run)?.as_ref())
        .map_err(AiError::from)?;
    candidate::validate_generated_research_context(&payload, facts.matrix_context(run)?.as_ref())
        .map_err(AiError::from)?;
    Ok(PreparedCandidate {
        kind: payload.kind(),
        canonical_json: serde_json::to_string(&payload)
            .map_err(|_| AiError::TemporaryUnavailable)?,
    })
}

pub(super) fn prepared_preview(
    db: &rusqlite::Connection,
    credentials: &crate::ai::JsonCredentialStore,
    draft: &crate::ai::generation_support::AiTaskSpecDraft,
) -> Result<crate::ai::generation_support::AiTaskPreview, crate::ai::error::AiError> {
    let (catalog, key) = actual_package();
    crate::runtime_plugins::literature_trace::service::analysis_workflow::preview_for_test(
        db,
        credentials,
        draft,
        &catalog,
        &key,
    )
}

fn prepared_template(
    db: &rusqlite::Connection,
    paper: &str,
    reference: &crate::ai::candidate_contracts::PaperTemplateRef,
) -> Result<crate::ai::candidate_contracts::AiOverviewProfileContext, crate::ai::error::AiError> {
    use crate::ai::{
        candidate_contracts::PaperTemplateRef, error::AiError, template_facts::TemplateFacts,
    };
    use research_literature_library_native::analysis_workflow::template::resolve;
    use research_workbench_native_sdk::analysis_workflow::template::TemplateInputV1;
    let facts = TemplateFacts::new(db);
    let input = if matches!(reference, PaperTemplateRef::Resource { .. }) {
        let (selection_revision, binding) = facts.reading_template(paper)?;
        resolve(&TemplateInputV1::ResourceBinding {
            reference: reference.clone(),
            selection_revision,
            binding,
        })?;
        TemplateInputV1::ResourceContent {
            paper_id: paper.into(),
            reference: reference.clone(),
            selection_revision,
            raw: facts.resource_revision(reference)?,
        }
    } else {
        resolve(&TemplateInputV1::LegacyAvailability {
            managed_resources: facts.managed_resources()?,
        })?;
        resolve(&TemplateInputV1::LegacyIdentity {
            reference: reference.clone(),
            latest: facts.latest_portrait(paper, "paper_portrait_v1")?,
        })?;
        facts.legacy_content(paper, reference)?
    };
    resolve(&input)?.ok_or(AiError::TemporaryUnavailable)
}

fn candidate_quotes(
    bundle: &crate::ai::candidate_contracts::AiEvidenceBundle,
) -> Vec<research_workbench_native_sdk::candidate_contracts::EvidenceQuote> {
    bundle
        .items
        .iter()
        .map(
            |item| research_workbench_native_sdk::candidate_contracts::EvidenceQuote {
                citation_key: item.citation_key.clone(),
                quote_exact: item.quote_exact.clone(),
            },
        )
        .collect()
}
#[cfg(all(debug_assertions, feature = "development-data"))]
fn prepared_revision(
    db: &rusqlite::Connection,
    run: &str,
    id: &str,
    kind: &str,
    bundle: &crate::ai::candidate_contracts::AiEvidenceBundle,
    value: Value,
) -> Result<crate::ai::generation_policy::PreparedCandidate, crate::ai::error::AiError> {
    use crate::ai::{
        error::AiError,
        generation_policy::{CandidateValidationFacts, PreparedCandidate},
    };
    use research_literature_library_native::{analysis_workflow::candidate, candidate_revision};
    let keys = bundle
        .items
        .iter()
        .map(|item| item.citation_key.clone())
        .collect();
    let payload = candidate::decode_candidate_value(value, &keys).map_err(AiError::from)?;
    candidate_revision::quotes_and_kind(&payload, kind, &candidate_quotes(bundle))
        .map_err(AiError::from)?;
    let facts = CandidateValidationFacts::new(db);
    candidate::validate_overview_profile(&payload, facts.overview_profile(run)?.as_ref())
        .map_err(AiError::from)?;
    candidate::validate_revised_research_context(&payload, facts.matrix_context(run)?.as_ref())
        .map_err(AiError::from)?;
    candidate_revision::confirmed_fields_unchanged(&payload, facts.active_confirmed_fields(id)?)
        .map_err(AiError::from)?;
    Ok(PreparedCandidate {
        kind: payload.kind(),
        canonical_json: candidate_revision::canonical_json(&payload).map_err(AiError::from)?,
    })
}

pub(super) fn defer_candidate(
    db: &mut rusqlite::Connection,
    draft: research_workbench_native_sdk::candidate_review::contracts::AiReviewDecisionDraft,
) -> Result<
    research_workbench_native_sdk::candidate_review::contracts::AiCandidate,
    crate::ai::error::AiError,
> {
    use crate::ai::{
        candidate_contracts::CandidatePayload,
        error::AiError,
        review_support::{self, GenericReviewAdapter, ReviewPlan, ReviewPromotion},
        structured_resource_validation::Flow,
    };
    use research_workbench_native_sdk::candidate_review::{
        contracts::AiCandidate,
        wire::{ReviewDecisionV1, ReviewInputV1},
    };
    type Failure = Flow<AiError, std::convert::Infallible>;
    struct Deferred;
    impl GenericReviewAdapter for Deferred {
        type Pause = std::convert::Infallible;
        fn review(&mut self, input: ReviewInputV1) -> Result<ReviewDecisionV1, Failure> {
            Ok(research_literature_library_native::candidate_review::decide(input))
        }
        fn decode(
            &mut self,
            candidate: &AiCandidate,
            value: Value,
        ) -> Result<CandidatePayload, Failure> {
            let keys = candidate
                .evidence_bundle
                .items
                .iter()
                .map(|item| item.citation_key.clone())
                .collect();
            research_literature_library_native::analysis_workflow::candidate::decode_candidate_value(value,&keys).map_err(|error| Flow::original(AiError::from(error)))
        }
        fn project_contains_paper(
            &mut self,
            _: &rusqlite::Connection,
            _: &str,
            _: &str,
        ) -> Result<bool, Failure> {
            panic!("deferred fixture has no project")
        }
        fn promote(
            &mut self,
            _: &rusqlite::Transaction<'_>,
            _: &ReviewPlan<'_>,
        ) -> ReviewPromotion<Self::Pause> {
            panic!("deferred fixture cannot promote")
        }
    }
    assert_eq!(draft.action, "defer");
    assert!(draft.project_id.is_none());
    review_support::decide_with(db, draft, true, &mut Deferred).map_err(Flow::into_static)
}

fn expected_metadata(db: &rusqlite::Connection, paper: &str) -> String {
    let (catalog, key) = actual_package();
    let package = catalog
        .discover()
        .unwrap()
        .packages
        .into_iter()
        .find(|package| package.package_key == key)
        .unwrap();
    let resources = crate::runtime_plugins::context_metadata::ContextMetadataResources::load(
        &catalog, &package,
    )
    .unwrap();
    let captured = resources.capture(db, paper).unwrap();
    research_literature_library_native::context_metadata::project(&captured.input).unwrap()
}

use super::*;
mod protocol;

pub(super) fn prepared_context(
    db: &rusqlite::Connection,
    paper: &str,
    attachments: &[String],
) -> Result<crate::ai::paper_reading_contracts::PreparedPaperContext, crate::ai::error::AiError> {
    use crate::runtime_plugins::literature_context::PaperContextResources;
    let (catalog, key) = actual_package();
    let package = catalog
        .discover()
        .unwrap()
        .packages
        .into_iter()
        .find(|package| package.package_key == key)
        .expect("exact selected context package");
    let resources = PaperContextResources::load(&catalog, &package)
        .map_err(|error| crate::ai::error::AiError::InvalidInput(error.to_string()))?;
    let captured = resources
        .capture_build(db, paper, attachments)
        .map_err(|error| {
            crate::ai::error::AiError::InvalidInput(error.failure.error.to_string())
        })?;
    Ok(protocol::build_context(&captured.input).into())
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
    let (selected, requested_key, selected_package) =
        match (std::env::var_os(ROOT_ENV), std::env::var_os(KEY_ENV)) {
            (Some(root), Some(key)) => {
                (PathBuf::from(root), Some(key.into_string().unwrap()), None)
            }
            (None, None) => {
                let package = PathBuf::from(
                    std::env::var_os("RESEARCH_ASSISTANT_TEST_LITERATURE_PACKAGE")
                        .expect("select explicit context root/key or a literature test package"),
                );
                assert!(fs::symlink_metadata(&package).unwrap().file_type().is_dir());
                let package = package.canonicalize().unwrap();
                assert!(package.starts_with(&temporary) && package != temporary);
                (package.parent().unwrap().to_path_buf(), None, Some(package))
            }
            _ => panic!("context metadata package root and key must be provided together"),
        };
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
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(root)).with_native_writing_preview(true));
    let listing = catalog.discover().unwrap();
    let key = requested_key.unwrap_or_else(|| {
        let snapshots = catalog.snapshots.lock().unwrap();
        let directory = selected_package.unwrap();
        listing
            .packages
            .iter()
            .find(|package| snapshots[&package.package_key].real_directory == directory)
            .expect("exact selected package directory must be present")
            .package_key
            .clone()
    });
    assert!(
        key.len() == 64
            && key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
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
    use research_workbench_native_sdk::{
        analysis_workflow::candidate::CandidateInputV1, paper_context_wire::encode_payload,
    };
    let keys = bundle
        .items
        .iter()
        .map(|item| item.citation_key.clone())
        .collect();
    let payload = protocol::decode_candidate_content(content, &keys).map_err(AiError::from)?;
    let encoded = encode_payload(&payload).map_err(|_| AiError::TemporaryUnavailable)?;
    let facts = CandidateValidationFacts::new(db);
    let (task_kind, suggestion_kind) = facts.run_output(run)?;
    protocol::validate_candidate(CandidateInputV1::KindAndQuotes {
        payload_cbor_base64: encoded.clone(),
        task_kind,
        suggestion_kind,
        evidence: candidate_quotes(bundle),
    })?;
    protocol::validate_candidate(CandidateInputV1::Profile {
        payload_cbor_base64: encoded.clone(),
        profile: facts.overview_profile(run)?,
    })?;
    let canonical_json = protocol::prepared(CandidateInputV1::Finalize {
        payload_cbor_base64: encoded,
        matrix: facts.matrix_context(run)?,
    })?;
    Ok(PreparedCandidate {
        kind: payload.kind(),
        canonical_json,
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
    use protocol::template as resolve;
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
    use research_workbench_native_sdk::{
        analysis_workflow::candidate::CandidateInputV1, paper_context_wire::encode_payload,
    };
    let keys = bundle
        .items
        .iter()
        .map(|item| item.citation_key.clone())
        .collect();
    let payload = protocol::decode_candidate_value(value, &keys)?;
    let encoded = encode_payload(&payload).map_err(|_| AiError::TemporaryUnavailable)?;
    protocol::validate_candidate(CandidateInputV1::RevisionQuotesAndKind {
        payload_cbor_base64: encoded.clone(),
        candidate_kind: kind.into(),
        evidence: candidate_quotes(bundle),
    })?;
    let facts = CandidateValidationFacts::new(db);
    protocol::validate_candidate(CandidateInputV1::Profile {
        payload_cbor_base64: encoded.clone(),
        profile: facts.overview_profile(run)?,
    })?;
    protocol::validate_candidate(CandidateInputV1::RevisionMatrix {
        payload_cbor_base64: encoded.clone(),
        matrix: facts.matrix_context(run)?,
    })?;
    let canonical_json = protocol::prepared(CandidateInputV1::RevisionConfirmedFields {
        payload_cbor_base64: encoded,
        active_fields: facts.active_confirmed_fields(id)?,
    })?;
    Ok(PreparedCandidate {
        kind: payload.kind(),
        canonical_json,
    })
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
    protocol::metadata(&captured.input).unwrap()
}

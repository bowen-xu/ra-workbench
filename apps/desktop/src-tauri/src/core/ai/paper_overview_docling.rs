//! Existing runtime entry dispatches only to the App-selected paper policy.
use super::{
    credential::JsonCredentialStore,
    error::AiCommandError,
    model::{AiRun, DoclingPaperOverviewDraft},
    provider::CancellationSignal,
};
use crate::{
    agent::{model::AgentModelResolver, service::AgentSessionService},
    storage::AppStore,
};
pub(super) async fn run_docling_paper_overview(
    store: AppStore,
    credentials: JsonCredentialStore,
    models: AgentModelResolver,
    draft: DoclingPaperOverviewDraft,
    cancellation: Option<CancellationSignal>,
    sessions: Option<std::sync::Arc<AgentSessionService>>,
) -> Result<AiRun, AiCommandError> {
    if let Some(binding) = store.native_note_trace().cloned() {
        let execution =
            super::docling_execution::DoclingExecution::new(store.clone(), models, sessions);
        let host = super::docling_execution::NativeDoclingHost {
            store,
            credentials,
            service: binding.service,
            key: binding.package_key,
            execution,
        };
        return tokio::spawn(async move {
            super::docling_execution::run_docling_paper_overview(&host, draft, cancellation).await
        })
        .await
        .map_err(|_| AiCommandError::from(super::error::AiError::TemporaryUnavailable))?;
    }
    (store.require_generation_policy_source()?.run_docling)(
        store,
        credentials,
        models,
        draft,
        cancellation,
        sessions,
    )
    .await
}
#[cfg(test)]
use super::{
    candidate_contracts::AiEvidenceBundle, generation_contracts::ProviderInput,
    reading_contracts::StoredImageAsset,
};
#[cfg(test)]
use super::{error::AiError, model::PaperAiReadingContextItem};
#[cfg(test)]
use rusqlite::Connection;
#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
fn input_has_images(input: &ProviderInput) -> bool {
    input.has_images()
}
#[cfg(test)]
pub(crate) fn persist_docling_evidence(
    db: &mut Connection,
    run: &AiRun,
    items: &[PaperAiReadingContextItem],
    assets: &BTreeMap<String, StoredImageAsset>,
) -> Result<AiEvidenceBundle, AiError> {
    macro_rules! sql {
        ($name:literal) => {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../tests/fixtures/native-resources-v1/literature/analysis-docling-evidence-",
                $name,
                ".v1.sql"
            ))
        };
    }
    let statements = super::docling_generation_support::DoclingStatements {
        existing: sql!("existing"),
        title: sql!("title"),
        bundle: sql!("bundle"),
        anchor: sql!("anchor"),
        item: sql!("item"),
        context: sql!("context"),
    };
    super::docling_generation_support::persist_docling_evidence(
        db,
        &statements,
        |run, item| {
            use research_workbench_native_sdk::analysis_workflow::docling::DoclingEvidenceRow;
            let (page_index, quote, context) = match item.id.as_str() {
                "P1-T1" => {
                    assert_eq!(item.page_index, Some(0));
                    assert!(item.regions.is_empty());
                    let text = item.text.clone().expect("authored text evidence");
                    (
                        0,
                        text.clone(),
                        format!("[P1-T1] kind=text page=1 rawRef=#/texts/0 text={text}"),
                    )
                }
                "P1-TABLE1" => {
                    assert_eq!(item.page_index, Some(2));
                    assert!(item.regions.is_empty());
                    (2, "table".into(), "[P1-TABLE1] kind=table page=3 rawRef=#/tables/0 label=table table={\"grid\":[[\"A\"]]}".into())
                }
                other => panic!("missing authored evidence response for {other}"),
            };
            Ok(Some(DoclingEvidenceRow {
                anchor_id: format!("docling:{run}:{}", item.id),
                page_index,
                page_label: (page_index + 1).to_string(),
                rects_json: "[]".into(),
                quote,
                context,
            }))
        },
        run,
        items,
        assets,
    )
}
#[cfg(test)]
mod tests {
    use super::super::paper_reading::{ProviderMessage, ProviderMessageRole};
    use rusqlite::{Connection, params};
    use serde_json::json;

    use super::*;
    const OVERVIEW_PROMPT_VERSION: &str = "paper-overview-docling-context-v3";
    use crate::ai::model::AiOverviewProfileContext;
    use crate::ai::paper_reading::ProviderImage;
    use crate::storage::migrate;

    #[test]
    fn modality_detection_keeps_text_requests_on_text_transport() {
        let text = ProviderInput {
            audit_text: "text".to_owned(),
            system_text: None,
            messages: vec![ProviderMessage {
                role: ProviderMessageRole::User,
                text: "text".to_owned(),
                images: Vec::new(),
            }],
        };
        let image = ProviderInput {
            audit_text: "image".to_owned(),
            system_text: None,
            messages: vec![ProviderMessage {
                role: ProviderMessageRole::User,
                text: "image".to_owned(),
                images: vec![ProviderImage {
                    context_item_id: "P1-F1".to_owned(),
                    context_text: "figure".to_owned(),
                    data_url: "data:image/png;base64,AAAA".to_owned(),
                }],
            }],
        };
        assert!(!input_has_images(&text));
        assert!(input_has_images(&image));
    }

    #[test]
    fn docling_evidence_keeps_page_level_table_items_without_regions() {
        let mut connection = database();
        insert_paper_fixture(&connection);
        let run = run_fixture(None);
        insert_run_fixture(&connection, &run);
        let item = PaperAiReadingContextItem {
            id: "P1-TABLE1".to_owned(),
            paper_id: "paper".to_owned(),
            attachment_id: "att".to_owned(),
            content_sha256: "a".repeat(64),
            parse_run_id: "parse".to_owned(),
            raw_ref: "#/tables/0".to_owned(),
            kind: "table".to_owned(),
            label: Some("table".to_owned()),
            page_index: Some(2),
            regions: Vec::new(),
            charspan: None,
            source_text_hash: Some("c".repeat(64)),
            text: None,
            table_json: Some(json!({"grid":[["A"]]})),
        };

        let bundle = persist_docling_evidence(&mut connection, &run, &[item], &BTreeMap::new())
            .expect("persist evidence");

        assert_eq!(bundle.items.len(), 1);
        assert_eq!(bundle.items[0].citation_key, "P1-TABLE1");
        assert_eq!(bundle.items[0].page_index, 2);
        assert_eq!(bundle.items[0].current_anchor_status, "page_level");
    }

    fn database() -> Connection {
        let mut connection = Connection::open_in_memory().expect("open test database");
        migrate(&mut connection).expect("migrate");
        connection
    }

    fn insert_paper_fixture(connection: &Connection) {
        connection
            .execute(
                "insert into bibliographic_items (id, item_type, title, created_at, updated_at)
                 values ('paper', 'journalArticle', 'Paper', 'now', 'now')",
                [],
            )
            .expect("paper");
        connection
            .execute(
                "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
                 values (?1, 'aa/paper.pdf', 'application/pdf', 10, 'now')",
                [&"a".repeat(64)],
            )
            .expect("content");
        connection
            .execute(
                "insert into attachments (id, item_id, content_sha256, display_name, created_at)
                 values ('att', 'paper', ?1, 'paper.pdf', 'now')",
                [&"a".repeat(64)],
            )
            .expect("attachment");
        connection
            .execute(
                "insert into docling_parse_runs
                 (id, item_id, attachment_id, content_sha256, service_base_url, docling_status,
                  parse_status, json_content, raw_response, created_at, finished_at)
                 values ('parse', 'paper', 'att', ?1, 'http://127.0.0.1:5001', 'success',
                         'success', '{}', '{}', 'now', 'now')",
                [&"a".repeat(64)],
            )
            .expect("parse run");
    }

    fn insert_run_fixture(connection: &Connection, run: &AiRun) {
        connection
            .execute(
                "insert into ai_runs
                 (id, task_kind, status, provider_id, model_id, pipeline_version, prompt_version,
                  output_schema_version, user_goal, scope_hash, input_item_count, max_input_tokens,
                  max_output_tokens, timeout_ms, created_at, updated_at)
                 values (?1, 'paper_overview', 'generating', 'fixture-generation', 'fixture-v1',
                         'pipeline', 'prompt', 1, 'goal', ?2, 1, 1000, 1000, 1000, 'now', 'now')",
                params![&run.id, &run.scope_hash],
            )
            .expect("run");
    }

    fn run_fixture(overview_profile: Option<AiOverviewProfileContext>) -> AiRun {
        AiRun {
            id: "run".to_owned(),
            task_kind: "paper_overview".to_owned(),
            status: "generating".to_owned(),
            provider_id: "fixture-generation".to_owned(),
            model_id: "fixture-v1".to_owned(),
            reasoning_effort: None,
            pipeline_version: "paper-overview-docling-runtime-v1".to_owned(),
            prompt_version: OVERVIEW_PROMPT_VERSION.to_owned(),
            output_schema_version: 1,
            user_goal: "goal".to_owned(),
            scope_hash: "f".repeat(64),
            input_item_count: 1,
            max_input_tokens: 64_000,
            max_output_tokens: 4_000,
            timeout_ms: 120_000,
            cloud_consent_version: None,
            cloud_consented_at: None,
            cancel_requested: false,
            input_tokens: None,
            output_tokens: None,
            cache_hit_tokens: None,
            cache_miss_tokens: None,
            error_code: None,
            error_message: None,
            created_at: "now".to_owned(),
            started_at: None,
            finished_at: None,
            updated_at: "now".to_owned(),
            scope_items: Vec::new(),
            stages: Vec::new(),
            overview_retry: None,
            overview_profile,
            suggestion_candidate_kind: None,
            matrix_synthesis_context: None,
            shared_conversation_id: None,
            shared_request_id: None,
            shared_turn_id: None,
            shared_turn_outcome: None,
            shared_candidate_id: None,
            shared_terminal_status: None,
        }
    }
}

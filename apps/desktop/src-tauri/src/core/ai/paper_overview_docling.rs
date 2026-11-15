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
    use research_literature_library_native::analysis_workflow::docling as rules;
    let statements = super::docling_generation_support::DoclingStatements {
        existing: rules::EXISTING_SQL,
        title: rules::TITLE_SQL,
        bundle: rules::BUNDLE_SQL,
        anchor: rules::ANCHOR_SQL,
        item: rules::ITEM_SQL,
        context: rules::CONTEXT_SQL,
    };
    super::docling_generation_support::persist_docling_evidence(
        db,
        &statements,
        |run, item| rules::evidence_row(run, item).map_err(Into::into),
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
    const RETRIEVAL_CONFIG_VERSION: &str = "docling-structured-context-v1";
    use crate::ai::model::{
        AiOverviewProfileContext, AiReadingProfileRef, PaperAiReadingRect, PaperAiReadingRegion,
    };
    use crate::ai::paper_reading::ProviderImage;
    use crate::storage::migrate;
    use research_literature_library_native::analysis_workflow::docling_prompt::build_overview_input;
    use research_workbench_native_sdk::{
        analysis_workflow::docling::DoclingPromptInputV1, candidate_contracts::EvidenceQuote,
    };

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
    fn overview_input_uses_docling_context_ids_as_evidence_ids() {
        let run = AiRun {
            id: "run".to_owned(),
            task_kind: "paper_overview".to_owned(),
            status: "generating".to_owned(),
            provider_id: "qwen".to_owned(),
            model_id: "qwen-vl".to_owned(),
            reasoning_effort: None,
            pipeline_version: "paper-overview-docling-runtime-v1".to_owned(),
            prompt_version: OVERVIEW_PROMPT_VERSION.to_owned(),
            output_schema_version: 1,
            user_goal: "goal".to_owned(),
            scope_hash: "scope".to_owned(),
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
            overview_profile: Some(AiOverviewProfileContext {
                paper_template: None,
                paper_id: "paper".to_owned(),
                selection_revision: 1,
                profile_refs: vec![AiReadingProfileRef {
                    profile_id: "generic_overview".to_owned(),
                    profile_version: 1,
                }],
                field_kinds: vec!["research_question".to_owned(), "approach".to_owned()],
            }),
            suggestion_candidate_kind: None,
            matrix_synthesis_context: None,
            shared_conversation_id: None,
            shared_request_id: None,
            shared_turn_id: None,
            shared_turn_outcome: None,
            shared_candidate_id: None,
            shared_terminal_status: None,
        };
        let region = PaperAiReadingRegion {
            page_index: 0,
            rect: PaperAiReadingRect {
                x: 0.1,
                y: 0.2,
                width: 0.3,
                height: 0.4,
            },
        };
        let items = vec![
            PaperAiReadingContextItem {
                id: "P1-T1".to_owned(),
                paper_id: "paper".to_owned(),
                attachment_id: "att".to_owned(),
                content_sha256: "a".repeat(64),
                parse_run_id: "parse".to_owned(),
                raw_ref: "#/texts/0".to_owned(),
                kind: "text".to_owned(),
                label: Some("paragraph".to_owned()),
                page_index: Some(0),
                regions: vec![region.clone()],
                charspan: Some([0, 10]),
                source_text_hash: Some("b".repeat(64)),
                text: Some("The paper asks a question.".to_owned()),
                table_json: None,
            },
            PaperAiReadingContextItem {
                id: "P1-T2".to_owned(),
                paper_id: "paper".to_owned(),
                attachment_id: "att".to_owned(),
                content_sha256: "a".repeat(64),
                parse_run_id: "parse".to_owned(),
                raw_ref: "#/texts/1".to_owned(),
                kind: "text".to_owned(),
                label: Some("paragraph".to_owned()),
                page_index: Some(0),
                regions: vec![region.clone()],
                charspan: Some([11, 20]),
                source_text_hash: Some("e".repeat(64)),
                text: Some("This unbundled context must not be sent.".to_owned()),
                table_json: None,
            },
            PaperAiReadingContextItem {
                id: "P1-F1".to_owned(),
                paper_id: "paper".to_owned(),
                attachment_id: "att".to_owned(),
                content_sha256: "a".repeat(64),
                parse_run_id: "parse".to_owned(),
                raw_ref: "#/pictures/0".to_owned(),
                kind: "picture".to_owned(),
                label: None,
                page_index: Some(0),
                regions: vec![region],
                charspan: None,
                source_text_hash: None,
                text: None,
                table_json: None,
            },
            PaperAiReadingContextItem {
                id: "P1-TABLE1".to_owned(),
                paper_id: "paper".to_owned(),
                attachment_id: "att".to_owned(),
                content_sha256: "a".repeat(64),
                parse_run_id: "parse".to_owned(),
                raw_ref: "#/tables/0".to_owned(),
                kind: "table".to_owned(),
                label: None,
                page_index: Some(0),
                regions: Vec::new(),
                charspan: None,
                source_text_hash: Some("c".repeat(64)),
                text: None,
                table_json: Some(json!({"grid":[["A"]]})),
            },
        ];
        let bundle = AiEvidenceBundle {
            id: "bundle".to_owned(),
            run_id: "run".to_owned(),
            scope_hash: "scope".to_owned(),
            retrieval_config_version: RETRIEVAL_CONFIG_VERSION.to_owned(),
            validation_status: "valid".to_owned(),
            created_at: "now".to_owned(),
            items: vec![evidence("P1-T1"), evidence("P1-F1"), evidence("P1-TABLE1")],
        };
        let mut assets = BTreeMap::new();
        assets.insert(
            "P1-F1".to_owned(),
            StoredImageAsset {
                context_item_id: "P1-F1".to_owned(),
                content_sha256: "d".repeat(64),
                data_url: "data:image/png;base64,AAAA".to_owned(),
            },
        );

        let input = build_overview_input(&DoclingPromptInputV1 {
            items,
            assets,
            profile: run.overview_profile,
            evidence: bundle
                .items
                .iter()
                .map(|item| EvidenceQuote {
                    citation_key: item.citation_key.clone(),
                    quote_exact: item.quote_exact.clone(),
                })
                .collect(),
        })
        .expect("input");

        assert!(
            input
                .audit_text
                .contains("\"allowedEvidenceItemIds\":[\"P1-T1\",\"P1-F1\",\"P1-TABLE1\"]")
        );
        assert!(input.audit_text.contains("\"outputJsonSkeleton\""));
        assert!(!input.audit_text.contains("filled | partial | not_found"));
        assert!(
            input
                .audit_text
                .contains("\"kind\":\"paper_overview_unit_v1\"")
        );
        assert!(
            input
                .audit_text
                .contains("The top-level kind must be exactly paper_overview_unit_v1")
        );
        assert!(input.audit_text.contains("evidenceItemIds"));
        assert!(input.audit_text.contains("[P1-TABLE1] kind=table"));
        assert!(!input.audit_text.contains("[P1-T2]"));
        assert_eq!(input.messages.len(), 1);
        assert_eq!(input.messages[0].images.len(), 1);
        assert_eq!(input.messages[0].images[0].context_item_id, "P1-F1");
        assert_eq!(
            input.messages[0].images[0].data_url,
            "data:image/png;base64,AAAA"
        );
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

    fn evidence(citation_key: &str) -> crate::ai::model::AiEvidenceItem {
        crate::ai::model::AiEvidenceItem {
            id: format!("ev-{citation_key}"),
            citation_key: citation_key.to_owned(),
            ordinal: 0,
            anchor_id: format!("anchor-{citation_key}"),
            paper_id: "paper".to_owned(),
            paper_title: "Paper".to_owned(),
            attachment_id: "att".to_owned(),
            content_sha256: "a".repeat(64),
            page_index: 0,
            page_label: Some("1".to_owned()),
            quote_exact: "quote".to_owned(),
            quote_prefix: String::new(),
            quote_suffix: String::new(),
            context_text: "context".to_owned(),
            relation_kind: "direct_quote".to_owned(),
            validation_status: "valid".to_owned(),
            current_anchor_status: "exact".to_owned(),
            created_at: "now".to_owned(),
        }
    }
}

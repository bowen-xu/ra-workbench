//! Real HTTP assertions after expansion by the production continuation owner.
use super::{
    compaction_tests::{compact_request, events, opaque},
    *,
};
use crate::agent::model::responses_continuation::ResponsesContinuation;
use research_agent_core::context::compaction_strategy::{CompactionKind, CompactionStrategy};

struct WindowSink(Arc<ResponsesContinuation>);

struct WireBytes;
impl research_agent_core::context::accounting::ContextTokenEstimator for WireBytes {
    fn estimate(
        &self,
        _: &[research_agent_core::model::ModelMessage],
        _: &[research_agent_core::model::ToolDefinition],
    ) -> Option<u64> {
        Some(1)
    }
    fn estimate_compaction(
        &self,
        output: &research_agent_core::context::compaction_strategy::CompactionOutput,
    ) -> Option<u64> {
        let research_agent_core::context::compaction_strategy::CompactionOutput::RemoteWindow(
            items,
        ) = output
        else {
            return None;
        };
        Some(serde_json::to_vec(items).unwrap().len() as u64)
    }
}
impl ResponsesReceiptSink for WindowSink {
    fn prepare_input(
        &self,
        _: Uuid,
        _: ModelIdentity,
        input: Vec<Value>,
    ) -> ModelFuture<'_, Vec<Value>> {
        Box::pin(async move { self.0.expand_compaction(&input, None) })
    }
    fn preserve(&self, _: ResponsesReceipt) -> ModelFuture<'_, ()> {
        Box::pin(async { panic!("compaction must not publish a sampling receipt") })
    }
}

#[tokio::test]
async fn both_paths_apply_injected_wire_budget_after_expansion_without_changing_window() {
    for legacy in [false, true] {
        let mut compact = compact_request();
        compact.source.messages = vec![research_agent_core::model::ModelMessage::System(
            "rules".into(),
        )];
        let original = vec![
            json!({"type":"function_call","call_id":"one","name":"read","arguments":"{}"}),
            json!({"type":"function_call","call_id":"two","name":"read","arguments":"{}"}),
            json!({"type":"function_call_output","call_id":"one","output":"large".repeat(100),"success":false}),
            json!({"type":"function_call_output","call_id":"two","output":"large".repeat(100),"success":true,"metadata":42}),
        ];
        let owner = Arc::new(
            ResponsesContinuation::new(Uuid::new_v4(), compact.source.model.clone(), MAX_REQUEST)
                .unwrap(),
        );
        owner
            .bind_window_sync(Uuid::from_u128(1), &compact.source.model, &original)
            .unwrap();
        let reply = if legacy {
            let body = json!({"output":[opaque()]}).to_string();
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        } else {
            ok(&events(&[opaque()], true))
        };
        let (url, peer) = server(reply, false).await;
        let native = Arc::new(transport(url, Arc::new(WindowSink(owner.clone()))));
        let accounting = Arc::new(
            research_agent_core::context::accounting::ContextAccounting {
                model: compact.source.model.clone(),
                budget: Some(
                    research_agent_core::context::compaction_strategy::CompactionBudget {
                        context_window_tokens: 1000,
                        effective_percent: 100,
                        configured_trigger_tokens: None,
                        output_reserve_tokens: 1,
                    },
                ),
                unknown_window_trigger_tokens: None,
                estimator: Arc::new(WireBytes),
            },
        );
        if legacy {
            compact.binding.kind = CompactionKind::RemoteWindow;
            ResponsesLegacyCompactionStrategy::new(native, compact.binding.clone())
                .unwrap()
                .with_input_accounting(accounting)
                .unwrap()
                .compact(compact.clone())
                .await
                .unwrap();
        } else {
            ResponsesV2CompactionStrategy::new(native, compact.binding.clone())
                .unwrap()
                .with_input_accounting(accounting)
                .unwrap()
                .compact(compact.clone())
                .await
                .unwrap();
        }
        let sent = peer.await.unwrap();
        let input = sent["input"].as_array().unwrap();
        assert!(
            input.contains(&original[2]),
            "rewrite stops once the producer window fits"
        );
        let last = input
            .iter()
            .find(|item| item["type"] == "function_call_output" && item["call_id"] == "two")
            .unwrap();
        assert_eq!(
            last["output"],
            "Output exceeded the available model context and was truncated"
        );
        assert_eq!(last["success"], true);
        assert_eq!(last["metadata"], 42);
        assert_eq!(owner.expand_compaction(&[], Some(0)).unwrap(), original);
    }
}

#[tokio::test]
async fn both_remote_paths_normalize_expanded_provider_window_without_rewriting_it() {
    for legacy in [false, true] {
        let mut compact = compact_request();
        compact
            .source
            .messages
            .push(research_agent_core::model::ModelMessage::User {
                content: vec![research_agent_core::model::ContentPart::Image {
                    media_type: "image/png".into(),
                    bytes: std::sync::Arc::from([1, 2, 3]),
                }],
                source_item_id: None,
            });
        let original = vec![
            json!({"type":"function_call","id":"fc_fixture","call_id":"missing","name":"read","arguments":"{}"}),
            json!({"type":"function_call_output","call_id":"orphan","output":"orphan raw result"}),
            json!({"type":"message","role":"user","content":[{"type":"input_image","image_url":"fixture image"},{"type":"input_audio","audio_url":"fixture audio"}]}),
            json!({"type":"tool_search_output","call_id":"server","execution":"server","tools":[{"name":"original tool"}]}),
            opaque(),
        ];
        let owner = Arc::new(
            ResponsesContinuation::new(Uuid::new_v4(), compact.source.model.clone(), MAX_REQUEST)
                .unwrap(),
        );
        owner
            .bind_window_sync(Uuid::from_u128(1), &compact.source.model, &original)
            .unwrap();
        let reply = if legacy {
            let value = json!({"output":[opaque()]});
            let body = value.to_string();
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        } else {
            ok(&events(&[opaque()], true))
        };
        let (url, peer) = server(reply, false).await;
        let mut native = transport(url, Arc::new(WindowSink(owner.clone())));
        native.config.capabilities.vision = false;
        assert!(
            matches!(
                native.request_body(&compact.source, false),
                Err(ModelError::UnsupportedCapability)
            ),
            "ordinary sampling keeps strict attachment validation"
        );
        if legacy {
            compact.binding.kind = CompactionKind::RemoteWindow;
            ResponsesLegacyCompactionStrategy::new(Arc::new(native), compact.binding.clone())
                .unwrap()
                .compact(compact.clone())
                .await
                .unwrap();
        } else {
            ResponsesV2CompactionStrategy::new(Arc::new(native), compact.binding.clone())
                .unwrap()
                .compact(compact.clone())
                .await
                .unwrap();
        }
        let sent = peer.await.unwrap();
        let input = sent["input"].as_array().unwrap();
        let call = input
            .iter()
            .position(|item| item["call_id"] == "missing")
            .unwrap();
        assert_eq!(input[call + 1]["type"], "function_call_output");
        assert_eq!(input[call + 1]["output"], "aborted");
        assert!(input[call + 1]["id"].as_str().unwrap().starts_with("fco_"));
        assert!(!input.iter().any(|item| item["call_id"] == "orphan"));
        let media = input
            .iter()
            .find(|item| {
                item["content"].is_array()
                    && item["content"][0]["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("image content omitted"))
            })
            .unwrap();
        assert!(
            media["content"][1]["text"]
                .as_str()
                .unwrap()
                .contains("audio content omitted")
        );
        assert!(input.contains(&original[3]));
        assert_eq!(
            input
                .iter()
                .filter(|item| item["content"]
                    .as_array()
                    .is_some_and(|parts| parts.iter().any(|part| part["text"]
                        == "image content omitted because you do not support image input")))
                .count(),
            2
        );
        assert!(
            matches!(compact.source.messages.last(), Some(research_agent_core::model::ModelMessage::User { content: parts, source_item_id: None }) if matches!(parts.first(),Some(research_agent_core::model::ContentPart::Image {..})))
        );
        assert!(input.contains(&original[4]));
        assert_eq!(
            input.last().unwrap()["type"] == "compaction_trigger",
            !legacy
        );
        assert_eq!(
            owner.expand_compaction(&[], Some(0)).unwrap(),
            original,
            "installed provider state must remain unchanged"
        );
    }
}

#[tokio::test]
async fn both_remote_paths_trim_search_and_custom_tail_but_preserve_user_boundary() {
    for legacy in [false, true] {
        let mut compact = compact_request();
        compact.source.messages = vec![research_agent_core::model::ModelMessage::System(
            "current rules".into(),
        )];
        let original = vec![
            json!({"type":"function_call","call_id":"earlier","name":"read","arguments":"{}"}),
            json!({"type":"custom_tool_call","call_id":"custom","name":"read","input":"evidence"}),
            json!({"type":"function_call_output","call_id":"earlier","output":"old evidence".repeat(100)}),
            json!({"type":"message","role":"user","content":[{"type":"input_text","text":"keep this boundary"}]}),
            json!({"type":"custom_tool_call_output","id":"custom-result","call_id":"custom","output":"tail evidence".repeat(100),"success":false,"metadata":{"version":"original"}}),
            json!({"type":"tool_search_output","id":"search-result","call_id":"search","execution":"server","status":"completed","tools":[{"name":"huge tool","description":"schema".repeat(100)}],"metadata":{"version":"original"}}),
        ];
        let owner = Arc::new(
            ResponsesContinuation::new(Uuid::new_v4(), compact.source.model.clone(), MAX_REQUEST)
                .unwrap(),
        );
        owner
            .bind_window_sync(Uuid::from_u128(1), &compact.source.model, &original)
            .unwrap();
        let reply = if legacy {
            let body = json!({"output":[opaque()]}).to_string();
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        } else {
            ok(&events(&[opaque()], true))
        };
        let (url, peer) = server(reply, false).await;
        let native = Arc::new(transport(url, Arc::new(WindowSink(owner.clone()))));
        let accounting = Arc::new(
            research_agent_core::context::accounting::ContextAccounting {
                model: compact.source.model.clone(),
                budget: Some(
                    research_agent_core::context::compaction_strategy::CompactionBudget {
                        context_window_tokens: 200,
                        effective_percent: 100,
                        configured_trigger_tokens: None,
                        output_reserve_tokens: 1,
                    },
                ),
                unknown_window_trigger_tokens: None,
                estimator: Arc::new(WireBytes),
            },
        );
        if legacy {
            compact.binding.kind = CompactionKind::RemoteWindow;
            ResponsesLegacyCompactionStrategy::new(native, compact.binding.clone())
                .unwrap()
                .with_input_accounting(accounting)
                .unwrap()
                .compact(compact.clone())
                .await
                .unwrap();
        } else {
            ResponsesV2CompactionStrategy::new(native, compact.binding.clone())
                .unwrap()
                .with_input_accounting(accounting)
                .unwrap()
                .compact(compact.clone())
                .await
                .unwrap();
        }
        let sent = peer.await.unwrap();
        let input = sent["input"].as_array().unwrap();
        assert!(
            input.contains(&original[2]),
            "user boundary protects earlier output even when still over budget"
        );
        assert!(input.contains(&original[3]));
        for (index, field, replacement) in [
            (
                4,
                "output",
                json!("Output exceeded the available model context and was truncated"),
            ),
            (5, "tools", json!([])),
        ] {
            let mut expected = original[index].clone();
            expected[field] = replacement;
            assert!(
                input.contains(&expected),
                "all non-payload fields must survive"
            );
        }
        assert_eq!(
            input.last().unwrap()["type"] == "compaction_trigger",
            !legacy
        );
        assert_eq!(owner.expand_compaction(&[], Some(0)).unwrap(), original);
    }
}

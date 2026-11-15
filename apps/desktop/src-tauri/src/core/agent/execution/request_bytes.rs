//! Content-byte guard shared by request preparation and transport retries.
use research_agent_core::model::{ContentPart, ModelMessage, ModelOutputItem, ToolCall};

pub(super) fn message_bytes(messages: &[ModelMessage]) -> Option<usize> {
    messages.iter().try_fold(0usize, |total, message| {
        let bytes = match message {
            ModelMessage::System(text) | ModelMessage::Developer(text) => text.len(),
            ModelMessage::ToolResult {
                call_id, content, ..
            } => call_id.len().checked_add(content.len())?,
            ModelMessage::User { content, .. } => content_bytes(content)?,
            ModelMessage::Assistant {
                content,
                tool_calls,
                ..
            } => tool_calls
                .iter()
                .try_fold(content_bytes(content)?, |sum, call| {
                    sum.checked_add(call_bytes(call)?)
                })?,
            ModelMessage::CanonicalOutput(item) => {
                item.validate().ok()?;
                let common = match &item.normalized {
                    Some(ModelOutputItem::Message(message)) => message.text.len(),
                    Some(ModelOutputItem::ReasoningSummary(summary)) => summary
                        .item_id
                        .len()
                        .checked_add(summary.text.len())?
                        .checked_add(serde_json::to_vec(&summary.parts).ok()?.len())?,
                    Some(ModelOutputItem::ToolCall(call)) => call
                        .call_id
                        .len()
                        .checked_add(call.name.len())?
                        .checked_add(call.raw_arguments_json.len())?
                        .checked_add(
                            call.namespace
                                .as_ref()
                                .map(|namespace| {
                                    serde_json::to_vec(namespace).ok().map(|bytes| bytes.len())
                                })
                                .unwrap_or(Some(0))?,
                        )?,
                    None => 0,
                };
                let native = item
                    .native
                    .as_ref()
                    .map(|native| native.to_json().ok().map(|json| json.len()))
                    .unwrap_or(Some(0))?;
                common.checked_add(native)?
            }
        };
        total.checked_add(bytes)
    })
}

fn content_bytes(parts: &[ContentPart]) -> Option<usize> {
    parts.iter().try_fold(0usize, |sum, part| {
        sum.checked_add(match part {
            ContentPart::Text(text) => text.len(),
            ContentPart::Image { bytes, .. } => bytes.len(),
        })
    })
}

fn call_bytes(call: &ToolCall) -> Option<usize> {
    call.call_id
        .len()
        .checked_add(call.name.len())?
        .checked_add(call.arguments_json().ok()?.len())?
        .checked_add(
            call.namespace
                .as_ref()
                .map(|namespace| serde_json::to_vec(namespace).ok().map(|bytes| bytes.len()))
                .unwrap_or(Some(0))?,
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::model::CompletedModelItem;

    #[test]
    fn canonical_and_legacy_calls_charge_the_same_original_arguments() {
        let raw = format!("{{{}\"v\":1}}", " ".repeat(1024));
        let call = ToolCall {
            namespace: None,
            call_id: "c".into(),
            name: "tool".into(),
            arguments: serde_json::json!({"v":1}),
            raw_arguments_json: Some(raw.clone()),
        };
        let expected = raw.len() + 5;
        assert_eq!(
            message_bytes(&[ModelMessage::Assistant {
                content: vec![],
                tool_calls: vec![call.clone()],
                phase: None
            }]),
            Some(expected)
        );
        assert_eq!(
            message_bytes(&[ModelMessage::CanonicalOutput(Box::new(
                CompletedModelItem {
                    canonical_item_id: None,
                    normalized: Some(ModelOutputItem::ToolCall((&call).try_into().unwrap())),
                    native: None
                }
            ))]),
            Some(expected)
        );
        let mut namespaced = call;
        namespaced.namespace = Some("命名🦀".into());
        let expected = expected
            + serde_json::to_vec(namespaced.namespace.as_ref().unwrap())
                .unwrap()
                .len();
        assert_eq!(
            message_bytes(&[ModelMessage::Assistant {
                content: vec![],
                tool_calls: vec![namespaced.clone()],
                phase: None,
            }]),
            Some(expected)
        );
        assert_eq!(
            message_bytes(&[ModelMessage::CanonicalOutput(Box::new(
                CompletedModelItem {
                    canonical_item_id: None,
                    normalized: Some(ModelOutputItem::ToolCall((&namespaced).try_into().unwrap())),
                    native: None,
                }
            ))]),
            Some(expected)
        );
    }

    #[test]
    fn common_namespace_bytes_are_charged_even_without_a_native_template() {
        let item = |namespace| {
            ModelMessage::CanonicalOutput(Box::new(CompletedModelItem {
                canonical_item_id: None,
                native: None,
                normalized: Some(ModelOutputItem::ToolCall(
                    research_agent_core::model::ToolCallIntent {
                        call_id: "c".into(),
                        name: "read".into(),
                        raw_arguments_json: "{".into(),
                        namespace,
                    },
                )),
            }))
        };
        let baseline = message_bytes(&[item(None)]).unwrap();
        let namespace = "命名🦀".repeat(100);
        assert_eq!(
            message_bytes(&[item(Some(namespace.clone()))]),
            Some(baseline + serde_json::to_vec(&namespace).unwrap().len())
        );
        assert_eq!(
            message_bytes(&[item(Some("\0".into()))]),
            Some(baseline + 8)
        );
    }
}

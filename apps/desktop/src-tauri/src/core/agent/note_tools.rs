//! Note proposals are inert structured results in the shared tool journal.
//! No editor, notes save command, filesystem or linked-resource expansion here.
use super::note_submission::{self, Receipt};
use crate::{
    notes::agent_rewrite::{self, InlineRewrite},
    storage::AppStore,
};
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolOutput},
    submission::SubmissionRecord,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(super) const REWRITE: &str = "research_propose_note_rewrite";
const MAX_ARGUMENT_BYTES: usize = 32 * 1024;
const MAX_RESULT_BYTES: usize = 128 * 1024;

pub(crate) struct NoteTools {
    pub store: AppStore,
    pub receipt: Receipt,
    pub record: SubmissionRecord,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RewriteArgs {
    source_id: String,
    edits: Vec<InlineRewrite>,
}

pub(crate) fn definitions() -> Vec<ToolDefinition> {
    let text = json!({"type":"object","properties":{"type":{"const":"text"},"text":{"type":"string"},"styles":{"type":"array","items":{"enum":["bold","italic","underline","strike","code"]}}},"required":["type","text","styles"],"additionalProperties":false});
    let inline = json!({"oneOf":[text.clone(),
        {"type":"object","properties":{"type":{"const":"link"},"href":{"type":"string"},"content":{"type":"array","items":text}},"required":["type","href","content"],"additionalProperties":false},
        {"type":"object","properties":{"type":{"const":"inlineFormula"},"source":{"type":"string"}},"required":["type","source"],"additionalProperties":false},
        {"type":"object","properties":{"type":{"const":"mention"},"targetKind":{"enum":["note_page","reading_note","paper","claim","manuscript"]},"targetId":{"type":"string"},"labelSnapshot":{"type":"string"}},"required":["type","targetKind","targetId","labelSnapshot"],"additionalProperties":false}
    ]});
    vec![ToolDefinition {
        namespace: None,
        name: REWRITE.into(),
        description: "Propose rewrites of 1–32 existing inline text blocks in a frozen note source. Supply complete inline content, preserving every link and mention target and its occurrence count. Read the frozen source first. Does not save or apply changes; the returned proposal remains unapplied. No asset, table, reference block or structure edits. Arguments <=32 KiB; preview <=128 KiB.".into(),
        parameters: json!({"type":"object","properties":{"sourceId":{"type":"string"},"edits":{"type":"array","minItems":1,"maxItems":32,"items":{"type":"object","properties":{"blockId":{"type":"string"},"content":{"type":"object","properties":{"kind":{"const":"inline"},"items":{"type":"array","items":inline}},"required":["kind","items"],"additionalProperties":false}},"required":["blockId","content"],"additionalProperties":false}}},"required":["sourceId","edits"],"additionalProperties":false}),
    }]
}

/// Reconstruct the same immutable result for execution and native review.
pub(super) fn evaluate(record: &SubmissionRecord, call: &ToolCall) -> Result<String, RunError> {
    if call.name != REWRITE || call.arguments.to_string().len() > MAX_ARGUMENT_BYTES {
        return Err(RunError::ToolRejected);
    }
    let args: RewriteArgs = crate::agent::tool_arguments::decode_object(
        &call.arguments,
        "Provide sourceId and edits using the declared note rewrite schema.",
    )?;
    let source = record
        .input
        .context
        .iter()
        .find(|s| s.source_id == args.source_id)
        .ok_or(RunError::ToolRejected)?;
    let edits = agent_rewrite::preview(source, &args.edits).map_err(|_| RunError::ToolRejected)?;
    let identity = json!([
        record.conversation_id,
        record.turn_id,
        record.context_snapshot_id,
        call.call_id,
        call.arguments
    ]);
    let result: Value = json!({"schemaVersion":1,"kind":"noteInlineRewrite","status":"proposed",
            "proposalId":format!("note-rewrite:{:x}", Sha256::digest(identity.to_string())),
            "conversationId":record.conversation_id,"turnId":record.turn_id,
            "contextSnapshotId":record.context_snapshot_id,"sourceId":source.source_id,
            "pageId":source.resource_id,"revision":source.revision,"anchor":source.anchor,"edits":edits});
    let output = result.to_string();
    if output.len() > MAX_RESULT_BYTES {
        return Err(RunError::ToolRejected);
    }
    Ok(output)
}
impl ToolExecutor for NoteTools {
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        evaluate(&self.record, call).map(|_| ())
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            // Binding can be revoked after the model was sent the source. The
            // tool must not produce a new proposal under that revoked scope.
            let saved = note_submission::restore(&self.store, self.receipt)
                .await
                .map_err(|_| RunError::ToolRejected)?;
            if saved != self.record {
                return Err(RunError::ToolRejected);
            }
            Ok(ToolOutput {
                content: evaluate(&self.record, &call)?,
                is_error: false,
            })
        })
    }
}

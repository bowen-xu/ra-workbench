//! Literal search of the already frozen source collection; no scope expansion.
use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    query: String,
    source_id: Option<String>,
    #[serde(default)]
    offset: usize,
}

pub(super) fn definition() -> ToolDefinition {
    ToolDefinition {
        namespace: None,
        name: SEARCH.into(),
        description: "Search frozen sources for a case-sensitive literal query (1–512 UTF-8 bytes), optionally within sourceId. Non-overlapping matches, 20 per page; follow nextOffset. Match and snippet positions count Unicode scalar values, compatible with research_read_source.".into(),
        parameters: json!({"type":"object","properties":{
            "query":{"type":"string","minLength":1,"maxLength":512},
            "sourceId":{"type":"string"},"offset":{"type":"integer","minimum":0}},
            "required":["query"],"additionalProperties":false}),
    }
}

pub(super) fn validate_arguments(arguments: &Value) -> Result<(), RunError> {
    let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| RunError::ToolArguments("Provide query (non-empty string, at most 512 UTF-8 bytes), optional sourceId (string), and offset (non-negative integer). No extra fields are accepted.".into()))?;
    if args.query.is_empty() || args.query.len() > 512 {
        return Err(RunError::ToolArguments("query must contain between 1 and 512 UTF-8 bytes; use a shorter literal search phrase.".into()));
    }
    Ok(())
}

pub(super) fn evaluate(record: &SubmissionRecord, arguments: &Value) -> Result<Value, RunError> {
    let args: Args =
        serde_json::from_value(arguments.clone()).map_err(|_| RunError::ToolRejected)?;
    if args.query.is_empty() || args.query.len() > 512 {
        return Err(RunError::ToolRejected);
    }
    if args.source_id.as_ref().is_some_and(|id| {
        !record
            .input
            .context
            .iter()
            .any(|source| &source.source_id == id)
    }) {
        return Err(RunError::ToolRejected);
    }
    let query_chars = args.query.chars().count();
    let mut total = 0usize;
    let mut matches = Vec::new();
    for source in &record.input.context {
        if args
            .source_id
            .as_ref()
            .is_some_and(|id| id != &source.source_id)
        {
            continue;
        }
        let mut previous_byte = 0;
        let mut previous_char = 0;
        for (byte, _) in source.text.match_indices(&args.query) {
            let start = previous_char + source.text[previous_byte..byte].chars().count();
            let end = start + query_chars;
            previous_byte = byte + args.query.len();
            previous_char = end;
            if total >= args.offset && matches.len() < 20 {
                let snippet_start = start.saturating_sub(32);
                let snippet: String = source
                    .text
                    .chars()
                    .skip(snippet_start)
                    .take(end - snippet_start + 64)
                    .collect();
                matches.push(json!({"sourceId":source.source_id,"kind":source.kind,
                    "resourceId":source.resource_id,"revision":source.revision,"anchor":source.anchor,
                    "startChar":start,"endChar":end,"snippetStartChar":snippet_start,"snippet":snippet}));
            }
            total += 1;
        }
    }
    if args.offset > total {
        return Err(RunError::ToolRejected);
    }
    let next = args.offset + matches.len();
    Ok(
        json!({"contextSnapshotId":record.context_snapshot_id,"turnId":record.turn_id,
        "total":total,"nextOffset":(next < total).then_some(next),"matches":matches}),
    )
}

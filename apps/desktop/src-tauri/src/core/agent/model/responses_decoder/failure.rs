//! Structured Responses errors; human-readable messages never select policy.
//! See the fixed Codex SSE/error boundary and current Responses error envelopes.
use super::*;

pub(super) fn decode(event: &Value, kind: &str) -> ModelError {
    let error = if kind == "response.failed" {
        &event["response"]["error"]
    } else if event["error"].is_object() {
        &event["error"]
    } else {
        event
    };
    match error["code"].as_str() {
        Some("context_length_exceeded") => ModelError::ContextWindowExceeded,
        Some("organization_usage_limit_exceeded") => ModelError::UsageLimitReached,
        Some("server_is_overloaded" | "server_overloaded") => ModelError::ServerOverloaded,
        Some("server_error") => ModelError::InternalServerError,
        Some("rate_limit_exceeded") => ModelError::RateLimited,
        Some("invalid_prompt" | "invalid_request") => ModelError::InvalidRequest,
        Some("invalid_api_key" | "authentication_error") => ModelError::Authentication,
        _ => ModelError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normal_and_compaction_streams_preserve_failure_categories_and_close() {
        for (code, expected) in [
            ("context_length_exceeded", ModelError::ContextWindowExceeded),
            (
                "organization_usage_limit_exceeded",
                ModelError::UsageLimitReached,
            ),
            ("server_is_overloaded", ModelError::ServerOverloaded),
            ("server_overloaded", ModelError::ServerOverloaded),
            ("server_error", ModelError::InternalServerError),
            ("rate_limit_exceeded", ModelError::RateLimited),
            ("invalid_prompt", ModelError::InvalidRequest),
            ("invalid_api_key", ModelError::Authentication),
            ("unknown", ModelError::Unavailable),
        ] {
            for compact in [false, true] {
                for envelope in 0..3 {
                    let mut decoder = if compact {
                        ResponsesDecoder::for_compaction("fixture".into(), 4096).unwrap()
                    } else {
                        ResponsesDecoder::new(
                            crate::agent::model::responses_native::NativeScope::fixture(),
                            4096,
                            1,
                        )
                        .unwrap()
                    };
                    decoder.decode(&json!({"type":"response.created","response":{"id":"r","model":"fixture"}}).to_string()).unwrap();
                    let error = json!({"code":code,"message":"Private provider content"});
                    let event = match envelope {
                        0 => json!({"type":"response.failed","response":{"error":error}}),
                        1 => json!({"type":"error","error":error}),
                        _ => json!({"type":"error","code":code}),
                    };
                    assert_eq!(decoder.decode(&event.to_string()).unwrap_err(), expected);
                    assert_eq!(
                        decoder.decode(&event.to_string()).unwrap_err(),
                        ModelError::InvalidStream
                    );
                    assert_eq!(
                        decoder.take_completed_output(),
                        Err(ModelError::IncompleteStream)
                    );
                }
            }
        }
    }

    #[test]
    fn messages_and_non_string_codes_cannot_claim_a_failure_category() {
        for code in [
            Value::Null,
            json!(17),
            json!({"code":"context_length_exceeded"}),
        ] {
            let event = json!({"type":"error","code":code,"message":"context_length_exceeded server_error"});
            assert_eq!(decode(&event, "error"), ModelError::Unavailable);
        }
    }
}

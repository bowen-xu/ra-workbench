//! Target-owned decoding for tools whose published schema requires an object.
//! The model decoder accepts raw strings; handlers select this boundary.
use research_agent_core::runner::RunError;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub(super) fn decode_object<T: DeserializeOwned>(
    arguments: &Value,
    guidance: &str,
) -> Result<T, RunError> {
    if !arguments.is_object() {
        return Err(RunError::ToolArguments(guidance.into()));
    }
    serde_json::from_value(arguments.clone()).map_err(|_| RunError::ToolArguments(guidance.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Arguments {
        count: u32,
    }

    #[test]
    fn declared_objects_reject_sequences_scalars_and_missing_fields_without_echoing_arguments() {
        for arguments in [
            json!([1]),
            json!(null),
            json!("private input"),
            json!(17),
            json!({}),
            json!({"count":"private input"}),
        ] {
            let result =
                decode_object::<Arguments>(&arguments, "Provide the required count integer.");
            assert!(
                matches!(result,Err(RunError::ToolArguments(message)) if message=="Provide the required count integer.")
            );
        }
        assert_eq!(
            decode_object::<Arguments>(&json!({"count":1}), "count")
                .unwrap()
                .count,
            1
        );
    }
}

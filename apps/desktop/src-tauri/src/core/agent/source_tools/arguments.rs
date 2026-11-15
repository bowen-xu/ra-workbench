//! Pure argument validation shared by live and historical source adapters.
//! Messages contain only tool contracts, never submitted source IDs or text.
use super::*;

pub(crate) fn validate_arguments(call: &ToolCall) -> Result<(), RunError> {
    let invalid = |message: &str| RunError::ToolArguments(message.into());
    if matches!(call.name.as_str(), LIST | READ | SEARCH) && !call.arguments.is_object() {
        return Err(invalid(
            "Source tools require an object matching their declared parameters.",
        ));
    }
    match call.name.as_str() {
        LIST => {
            serde_json::from_value::<ListArgs>(call.arguments.clone()).map_err(|_| invalid("offset must be a non-negative integer; omit it for the first page. No extra fields are accepted."))?;
        }
        READ => {
            let args = serde_json::from_value::<ReadArgs>(call.arguments.clone()).map_err(|_| invalid("Provide sourceId (string), startChar (non-negative integer, default 0), and maxChars (integer 1–8000, default 2000). No extra fields are accepted."))?;
            if args.source_id.trim().is_empty() {
                return Err(invalid(
                    "sourceId must be a non-empty string from research_list_sources.",
                ));
            }
            if args.max_chars == 0 || args.max_chars > 8000 {
                return Err(invalid(
                    "maxChars must be between 1 and 8000 Unicode characters. Read at most 8000, then use the returned nextChar as startChar to read the next page.",
                ));
            }
        }
        SEARCH => search::validate_arguments(&call.arguments)?,
        _ => return Err(RunError::ToolRejected),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call(arguments: Value) -> ToolCall {
        ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "fixture".into(),
            name: READ.into(),
            arguments,
        }
    }
    #[test]
    fn oversized_reads_return_contract_guidance_without_echoing_source_identity() {
        for max in [0, 8001, 13500] {
            let result = validate_arguments(&call(
                json!({"sourceId":"private source identity","maxChars":max}),
            ));
            let Err(RunError::ToolArguments(message)) = result else {
                panic!("recoverable argument error")
            };
            assert!(message.contains("8000") && message.contains("nextChar"));
            assert!(!message.contains("private source identity"));
        }
        for max in [1, 8000] {
            validate_arguments(&call(json!({"sourceId":"source","maxChars":max}))).unwrap();
        }
    }
    #[test]
    fn malformed_arguments_are_recoverable_but_unknown_tools_are_not() {
        for args in [
            json!({"sourceId":"source","maxChars":"8000"}),
            json!({"sourceId":"source","startChar":-1}),
            json!({"sourceId":"source","extra":true}),
        ] {
            assert!(matches!(
                validate_arguments(&call(args)),
                Err(RunError::ToolArguments(_))
            ));
        }
        let mut unknown = call(json!({}));
        unknown.name = "unknown".into();
        assert!(matches!(
            validate_arguments(&unknown),
            Err(RunError::ToolRejected)
        ));
    }
}

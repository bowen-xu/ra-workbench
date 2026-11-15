//! The native Responses edge already executes only the authoritative done call.
use super::*;

#[test]
fn streamed_function_metadata_and_arguments_are_drafts_until_output_item_done() {
    for (name, namespace, raw) in [
        ("write", None, r#" {"value":"done"} "#),
        ("read", Some("foreign::"), "{"),
        ("read", Some(""), ""),
    ] {
        let mut decoder = decoder();
        let mut response = ResponseAssembler::new(StreamLimits {
            text_bytes: 4096,
            argument_bytes: 4096,
            tool_calls: 2,
        })
        .unwrap();
        for event in start(&mut decoder) {
            response.push(event).unwrap();
        }
        let mut added = call();
        added["arguments"] = json!(r#"{"value":"draft"}"#);
        added["namespace"] = json!("draft::");
        assert!(
            decoder
                .decode(&json!({"type":"response.output_item.added","item":added}).to_string())
                .unwrap()
                .is_empty()
        );
        for event in [
            json!({"type":"response.function_call_arguments.delta","item_id":"f1","delta":"untrusted draft {"}),
            json!({"type":"response.function_call_arguments.done","item_id":"f1","arguments":"another draft"}),
        ] {
            assert!(decoder.decode(&event.to_string()).unwrap().is_empty());
        }
        let mut final_item = call();
        final_item["name"] = json!(name);
        final_item["arguments"] = json!(raw);
        if let Some(namespace) = namespace {
            final_item["namespace"] = json!(namespace);
        }
        let events = decoder.decode(&done(final_item.clone())).unwrap();
        assert!(
            matches!(events.as_slice(),[event]if completed_call(event).is_some_and(|call|
            call.call_id=="call1"&&call.name==name&&call.namespace.as_deref()==namespace&&call.raw_arguments_json==raw))
        );
        for event in events {
            response.push(event).unwrap();
        }
        for event in decoder
            .decode(&completed(vec![final_item.clone()]))
            .unwrap()
        {
            response.push(event).unwrap();
        }
        let result = response.finish().unwrap();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].call_id, "call1");
        assert_eq!(result.tool_calls[0].name, name);
        assert_eq!(result.tool_calls[0].namespace.as_deref(), namespace);
        assert_eq!(result.tool_calls[0].raw_arguments_json, raw);
        assert_eq!(decoder.take_completed_output().unwrap(), vec![final_item]);
    }
}

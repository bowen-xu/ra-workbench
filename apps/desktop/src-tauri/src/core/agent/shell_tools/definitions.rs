use super::*;
use research_agent_core::model::ToolDefinition;
use serde_json::json;

impl ShellTools {
    /// Register only after the host has admitted this frozen execution scope.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let id = json!({"type":"string","format":"uuid"});
        let wait = json!({"type":"integer","minimum":0,"maximum":60000});
        let action = |name: &str, mut properties: serde_json::Value, fields: Vec<&str>| {
            properties["action"] = json!({"const":name,"type":"string"});
            properties["process_id"] = id.clone();
            let mut required = vec!["action", "process_id"];
            required.extend(fields);
            json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
        };
        vec![ToolDefinition {
            namespace: None,
            name: EXEC.into(),
            description: "Execute a shell command in the host-authorized working copy. Returns cumulative output and a processId while running. Poll for final output. No caller-selected environment or filesystem scope. terminal enables PTY semantics. Completed handles are released after the result is recorded.".into(),
            parameters: json!({"type":"object","properties":{"command":{"type":"string","minLength":1,"maxLength":65536},
                "terminal":{"type":"boolean"},"yield_ms":wait},"required":["command"],"additionalProperties":false}),
        }, ToolDefinition {
            namespace: None,
            name: PROCESS.into(),
            description: "Control an owned process. poll returns cumulative output, not a delta. write reports the exact accepted byte count; never replay the accepted prefix. close_input sends EOF only for pipes; terminal EOF requires the terminal control character. cancel requests termination; poll to confirm completion. Head/tail output is separated when bytes were omitted.".into(),
            parameters: json!({"type":"object","oneOf":[
                action("poll",json!({"yield_ms":wait}),vec!["yield_ms"]),
                action("write",json!({"text":{"type":"string","maxLength":65536}}),vec!["text"]),
                action("close_input",json!({}),vec![]),action("cancel",json!({}),vec![])
            ]}),
        }]
    }
}

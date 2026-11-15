use super::*;

impl McpRuntime {
    /// Host-only optional contributor; no server creation or approval bypass.
    pub(crate) async fn thread_hint(&self, conversation: uuid::Uuid) -> Option<String> {
        let adapter = self
            .tools
            .iter()
            .find(|tool| tool.server_id() == "notes" && tool.is_approved())?;
        adapter.thread_hint(conversation).await.ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, path::PathBuf};
    #[tokio::test]
    async fn approved_notes_hint_uses_host_metadata_and_no_arguments() {
        let script = r#"while IFS= read line; do case "$line" in *initialize*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;; *tools/list*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"thread_hint","inputSchema":{"type":"object"}}]}}' ;; *tools/call*) case "$line" in *'"arguments"'*) exit 1 ;; *'"threadId":"00000000-0000-0000-0000-000000000001"'*) printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"text":" saved notes "},{"type":"image","data":"ignored"}]}}' ;; *) exit 1 ;; esac ;; esac; done"#;
        let server = ConfiguredServer {
            server_id: "notes".into(),
            environment_id: "project".into(),
            revision: "r1".into(),
            enabled: true,
            transport: Transport::Stdio {
                program: PathBuf::from("/bin/sh"),
                arguments: vec![OsString::from("-c"), OsString::from(script)],
                directory: std::env::temp_dir(),
            },
            environment: BTreeMap::new(),
            credential_handle: None,
        };
        let runtime = McpRuntime::build(
            vec![server],
            &HashSet::from(["notes".into()]),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(
            runtime
                .thread_hint(uuid::Uuid::from_u128(1))
                .await
                .as_deref(),
            Some(" saved notes ")
        );
    }
}

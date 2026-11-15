use super::*;
use std::{ffi::OsString, path::Path};

fn server(id: &str, environment: &str, receipt: &Path) -> ConfiguredServer {
    let script = r#"
id=2
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' initialized >> "$1"
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"Synthetic PID identity","inputSchema":{"type":"object"}}]}}' ;;
    *'"method":"tools/call"'*)
      id=$((id + 1))
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"server_pid=%s"}],"isError":false}}\n' "$id" "$$" ;;
  esac
done
"#;
    ConfiguredServer {
        server_id: id.into(),
        environment_id: environment.into(),
        revision: "r1".into(),
        transport: Transport::Stdio {
            program: "/bin/sh".into(),
            arguments: vec![
                OsString::from("-c"),
                OsString::from(script),
                OsString::from("fixture"),
                receipt.as_os_str().to_owned(),
            ],
            directory: std::env::temp_dir(),
        },
        environment: BTreeMap::new(),
        credential_handle: None,
        enabled: true,
    }
}

fn call(server: &str, id: &str) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: id.into(),
        name: format!("{server}__echo"),
        arguments: serde_json::json!({}),
    }
}

#[tokio::test]
async fn two_consumers_share_one_connection_and_leaving_does_not_restart_or_revoke_it() {
    let directory = tempfile::tempdir().unwrap();
    let first_receipt = directory.path().join("first");
    let second_receipt = directory.path().join("second");
    let identity = uuid::Uuid::new_v4().to_string();
    let first_id = format!("first-{identity}");
    let second_id = format!("second-{identity}");
    // Identical transport programs remain distinct configured identities.
    let runtime = Arc::new(
        McpRuntime::build(
            vec![
                server(&first_id, &identity, &first_receipt),
                server(&second_id, &identity, &second_receipt),
            ],
            &HashSet::from([first_id.clone(), second_id.clone()]),
            Duration::from_secs(2),
        )
        .await
        .unwrap(),
    );
    let one = runtime.scoped(HashSet::from([first_id.clone(), second_id.clone()]));
    let two = runtime.scoped(HashSet::from([first_id.clone()]));
    assert_eq!(one.definitions().len(), 2);
    assert_eq!(two.definitions().len(), 1);
    assert_eq!(
        two.resolve_name(&format!("{second_id}__echo")),
        ToolNameResolution::Registered
    );
    assert!(two.validate(&call(&second_id, "unselected")).is_err());
    let first_pid = one
        .authorize_and_execute(call(&first_id, "one-call"))
        .await
        .unwrap()
        .content;
    let distinct_pid = one
        .authorize_and_execute(call(&second_id, "second-identity"))
        .await
        .unwrap()
        .content;
    assert_ne!(first_pid, distinct_pid);
    drop(one);
    drop(runtime);
    let shared_pid = two
        .authorize_and_execute(call(&first_id, "two-call"))
        .await
        .unwrap()
        .content;
    assert_eq!(first_pid, shared_pid);
    // Arc unwinding also releases these handles on assertion failure. The
    // existing PipeProcess owner kills its own process group on final Drop.
    drop(two);
    assert_eq!(
        std::fs::read_to_string(&first_receipt).unwrap(),
        "initialized\n"
    );
    assert_eq!(
        std::fs::read_to_string(&second_receipt).unwrap(),
        "initialized\n"
    );
}

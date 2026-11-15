use super::*;
use std::collections::HashSet;

#[tokio::test]
async fn scoped_patch_rejects_read_only_unselected_and_new_files_before_workspace_access() {
    let root = tempfile::tempdir().unwrap();
    let tools = PatchTools {
        store: AppStore::for_test(
            rusqlite::Connection::open_in_memory().unwrap(),
            root.path().join("objects"),
        ),
        receipt: WritingSubmissionReceipt {
            conversation_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            manuscript_id: Uuid::new_v4(),
        },
        context_snapshot_id: Uuid::new_v4(),
        allowed_paths: Some(HashSet::from(["main.tex".into()])),
        scope: None,
    };
    let call = |path: &str| ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "permission-test".into(),
        name: APPLY_PATCH.into(),
        arguments: json!({"operations":[{"path":path,"before":null,"after":"text"}]}),
    };
    assert!(tools.validate(&call("main.tex")).is_ok());
    for path in ["readonly.tex", "unselected.tex", "new.tex"] {
        assert!(matches!(
            tools.validate(&call(path)),
            Err(RunError::ToolRejected)
        ));
        // No workspace exists: executing past the guard would return an unavailable
        // workspace result rather than reject the call before opening storage.
        assert!(matches!(
            tools.authorize_and_execute(call(path)).await,
            Err(RunError::ToolRejected)
        ));
    }
}

#[test]
fn whole_project_patch_allows_new_text_files_but_honors_read_only_and_excluded_paths() {
    use crate::agent::context_session::sources::{Access, Part, Resource};
    let root = tempfile::tempdir().unwrap();
    let manuscript = Uuid::new_v4();
    let part = |path: &str, selected: bool| Part {
        resource: Resource::ManuscriptFile {
            resource_id: manuscript,
            path: path.into(),
        },
        label: path.into(),
        selected,
        access: Access::ReadOnly,
    };
    let tools = PatchTools {
        store: AppStore::for_test(
            rusqlite::Connection::open_in_memory().unwrap(),
            root.path().join("objects"),
        ),
        receipt: WritingSubmissionReceipt {
            conversation_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            manuscript_id: manuscript,
        },
        context_snapshot_id: Uuid::new_v4(),
        allowed_paths: Some(HashSet::new()),
        scope: Some(Resource::Manuscript {
            resource_id: manuscript.to_string(),
            new_member_access: Some(Access::ReadWrite),
            group_access: None,
            parts: vec![part("readonly.tex", true), part("excluded.tex", false)],
        }),
    };
    let call = |path: &str| ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "scope".into(),
        name: APPLY_PATCH.into(),
        arguments: json!({"operations":[{"path":path,"before":null,"after":"text"}]}),
    };
    for path in ["new.tex", "sections/new.tex", "STYLE.STY"] {
        assert!(tools.validate(&call(path)).is_ok(), "{path}");
    }
    for path in [
        "refs.bib",
        "sources/LOCAL.BIB",
        "readonly.tex",
        "excluded.tex",
        "../outside.tex",
        "/outside.tex",
        "secret.key",
    ] {
        assert!(tools.validate(&call(path)).is_err(), "{path}");
    }
}

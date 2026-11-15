//! Bind restoration to an independently validated original application and undo.
use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    original_scope: Scope,
    request: restoration::Request,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    original_scope: Scope,
    result: restoration::Response,
}

#[cfg(target_os = "macos")]
pub(crate) async fn execute(
    store: &crate::storage::AppStore,
    request: Request,
) -> Result<Response, Error> {
    let original = request.original_scope.checked()?;
    let result = restoration::execute_for(store, request.request, Some(original)).await?;
    Ok(Response {
        original_scope: request.original_scope,
        result,
    })
}

#[tauri::command]
pub(crate) async fn agent_writing_undo_editor_restoration(
    store: tauri::State<'_, crate::storage::AppStore>,
    request: Request,
) -> Result<Response, Error> {
    #[cfg(target_os = "macos")]
    {
        execute(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn undo_restoration_rejects_unknown_fields_at_both_levels() {
        let scope = serde_json::json!({"conversationId":Uuid::new_v4(),"manuscriptId":Uuid::new_v4(),"proposalId":Uuid::new_v4(),"operationId":Uuid::new_v4()});
        let valid = serde_json::json!({"originalScope":scope,"request":{"scope":scope,"action":"read","path":"main.tex"}});
        assert!(serde_json::from_value::<Request>(valid.clone()).is_ok());
        let mut outer = valid.clone();
        outer["extra"] = true.into();
        let mut inner = valid.clone();
        inner["request"]["extra"] = true.into();
        let mut action = valid;
        action["request"]["action"] = "apply".into();
        for value in [outer, inner, action] {
            assert!(serde_json::from_value::<Request>(value).is_err());
        }
    }
}

//! One approved lazy owner supplies shell, file inspection and compilation.
use super::*;
pub(super) async fn compose(
    store: AppStore,
    record: &SubmissionRecord,
    sessions: Arc<AgentSessionService>,
    workspace: workspace::WorkspaceTools,
) -> Result<
    (
        workspace::WorkspaceTools,
        Option<crate::agent::shell_tools::ShellTools>,
        Option<compile_workspace::CompileWorkspaceTools>,
    ),
    WritingSubmissionError,
> {
    if crate::agent::context_session::permissions::writing_paths(record).is_some() {
        return Ok((workspace, None, None));
    }
    let Some((shell, host)) =
        shell::for_submission(store.clone(), record.clone(), sessions.clone()).await?
    else {
        return Ok((workspace, None, None));
    };
    let compiler = compile_workspace::CompileWorkspaceTools::for_submission(
        store,
        record,
        sessions,
        host.clone(),
    )
    .await?;
    Ok((workspace.with_shell_host(host), Some(shell), compiler))
}

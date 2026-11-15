//! Original sandbox, bounded LaTeX flags and process-registry admission.
use super::*;

pub(super) struct PreparedLaunch {
    pub(super) process_id: Uuid,
    pub(super) output_directory: PathBuf,
}

pub(super) fn prepare_launch(
    owner: &CompileWorkspaceTools,
    lease: &mut WorkspaceLease,
    request: Request,
    result_id: Uuid,
    toolchain: LatexToolchainInfo,
    compiler: &compiler::Compiler,
) -> Result<PreparedLaunch, RunError> {
    let work = lease
        .working_directory()
        .map_err(|_| RunError::ToolRejected)?;
    if !work.join(&request.root_path).is_file() {
        return Err(RunError::ToolRejected);
    }
    let output_directory = work.join(format!(".research-assistant-compile-{result_id}"));
    let recipe = compiler.recipe(
        &toolchain,
        &LatexWorkspaceCompileRequest {
            root_path: &request.root_path,
            engine: &request.engine,
            draft: request.draft,
            stop_on_first_error: request.stop_on_first_error,
            from_scratch: request.from_scratch,
            output_directory: &output_directory,
        },
    )?;
    let spec = ProcessSpec {
        program: recipe.program,
        arguments: recipe.arguments,
        directory: work.clone(),
        environment: BTreeMap::from([
            (OsString::from("PATH"), OsString::from(recipe.path)),
            (OsString::from("NO_COLOR"), OsString::from("1")),
        ]),
    };
    let sandboxed = prepare_offline_sandbox(
        spec,
        OfflineSandboxPaths {
            work: work.clone(),
            read_only: recipe.read_only,
            protected: protected_paths(&owner.store, &work)?,
            allow_system_metadata: true,
        },
    )
    .map_err(|_| RunError::ToolRejected)?;
    fs::create_dir(&output_directory).map_err(|_| RunError::EventSink)?;
    let mut launcher_entered = false;
    let process_result = (|| {
        let canonical = output_directory
            .canonicalize()
            .map_err(|_| RunError::EventSink)?;
        if canonical != output_directory || canonical.parent() != Some(work.as_path()) {
            return Err(RunError::ToolRejected);
        }
        let db = owner.store.connection().map_err(|_| RunError::EventSink)?;
        owner.authorize_owned(&db, lease)?;
        owner
            .registry
            .start(owner.scope.clone(), || {
                launcher_entered = true;
                let execution = lease
                    .begin_execution()
                    .map_err(|_| crate::agent::process::ProcessError::ResourceRelease)?;
                execution.spawn(
                    sandboxed,
                    ProcessLimits {
                        retained_bytes: 128 * 1024,
                        execution_timeout: Duration::from_secs(150),
                        drain_timeout: Duration::from_secs(5),
                    },
                    None,
                )
            })
            .map_err(|_| RunError::EventSink)
    })();
    let process_id = match process_result {
        Ok(process_id) => process_id,
        Err(error) if launcher_entered => {
            // The launcher ran and may have left an execution marker for
            // recovery. Keep the work copy in that case.
            return Err(error);
        }
        Err(error) => {
            // Admission failed before the closure could create an execution
            // marker. Do not leave an unreviewable generated directory behind.
            owner.cleanup_output(&output_directory, result_id)?;
            return Err(error);
        }
    };
    Ok(PreparedLaunch {
        process_id,
        output_directory,
    })
}

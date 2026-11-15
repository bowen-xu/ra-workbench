//! Original host discovery rechecks Core authorization and the borrowed file rows.
use super::{
    context::Binding,
    data_failure, decided_failure, failure,
    files::resources::{load_file_list, read_file_list},
};
use crate::{
    agent::resource_catalog::{self, CatalogRead, Error, Request, Resource},
    runtime_plugins::native::{DecidedProject, NativeProfile, package},
    storage::AppStore,
};
use research_workbench_native_sdk::writing_project::{Decision, Input};
use std::sync::Arc;

pub(crate) async fn list(
    binding: &Binding,
    store: AppStore,
    request: Request,
) -> Result<Vec<Resource>, Error> {
    let checked_store = store.clone();
    let checked_request = request.clone();
    tokio::task::spawn_blocking(move || {
        resource_catalog::authorize(&checked_store, &checked_request)
    })
    .await
    .map_err(|_| Error::Unavailable)??;
    let _operation = binding
        .service
        .project_operation(&binding.package_key)
        .await
        .map_err(Error::Native)?;
    let key = binding.package_key.clone();
    let catalog = Arc::clone(&binding.catalog);
    let selected = key.clone();
    let resource = tokio::task::spawn_blocking(move || {
        let package = package::accept(&catalog, &selected, NativeProfile::WritingProjects)?;
        load_file_list(&catalog, &package)
            .map_err(|error| failure(error.code, error.message, &selected))
    })
    .await
    .map_err(|_| {
        Error::Native(failure(
            "plugin_host_accept_failed",
            "写作资料列表资源核对任务未完成。",
            &key,
        ))
    })?
    .map_err(Error::Native)?;
    let captured_resource = resource.clone();
    let captured_store = store.clone();
    let captured_request = request.clone();
    let captured = tokio::task::spawn_blocking(move || {
        resource_catalog::list_with_files(&captured_store, &captured_request, &mut |db, id| {
            let rows = read_file_list(db, &captured_resource, id)
                .map_err(data_failure)
                .map_err(Error::Native)?;
            Ok(CatalogRead::Need(Box::new(Input::HostFiles {
                manuscript_id: id.into(),
                rows,
            })))
        })
    })
    .await
    .map_err(|_| Error::Unavailable)??;
    let input = match captured {
        CatalogRead::Ready(resources) => return Ok(resources),
        CatalogRead::Need(input) => *input,
    };
    let DecidedProject {
        output,
        instance_id,
        request_id,
    } = binding
        .service
        .project_decision(key.clone(), input.clone())
        .await
        .map_err(Error::Native)?;
    let mismatch = || {
        Error::Native(decided_failure(
            "plugin_host_response_mismatch",
            "写作资料列表结果未保持原读取身份。",
            &key,
            &instance_id,
            &request_id,
        ))
    };
    if output.input != input {
        return Err(mismatch());
    }
    let Decision::HostFiles { items } = output.decision else {
        return Err(mismatch());
    };
    let Input::HostFiles {
        manuscript_id,
        rows,
    } = input
    else {
        return Err(mismatch());
    };
    let selected = key.clone();
    let captured_instance = instance_id.clone();
    let captured_request = request_id.clone();
    let finished = tokio::task::spawn_blocking(move || {
        resource_catalog::list_with_files(&store, &request, &mut |db, id| {
            let mut result = read_file_list(db, &resource, id).map_err(data_failure);
            if result
                .as_ref()
                .is_ok_and(|current| current != &rows || id != manuscript_id)
            {
                let mut error = failure(
                    "plugin_data_snapshot_changed",
                    "写作资料在读取期间已变化，请重新读取。",
                    &selected,
                );
                error.data_error = Some(Box::new(
                    crate::data_interface::plugin_resources::PluginDataError {
                        code: "plugin_data_snapshot_changed",
                        message: "写作资料在读取期间已变化，请重新读取。",
                        package_key: selected.clone(),
                        resource_id: resource.grant().id.clone(),
                        resource_version: 1,
                        phase: "recheck",
                        safe_cause: "writing_host_files_read_set_changed".into(),
                        sqlite_code: None,
                        sqlite_extended_code: None,
                        original_sql_failure: None,
                    },
                ));
                result = Err(error);
            }
            result.map_err(|mut error| {
                error.instance_id = Some(captured_instance.clone());
                error.request_id = Some(captured_request.clone());
                Error::Native(error)
            })?;
            Ok(CatalogRead::Ready(items.clone()))
        })
    })
    .await
    .map_err(|_| {
        Error::Native(decided_failure(
            "storage_error",
            "写作资料列表读取核对任务未完成。",
            &key,
            &instance_id,
            &request_id,
        ))
    })??;
    match finished {
        CatalogRead::Ready(resources) => Ok(resources),
        CatalogRead::Need(_) => Err(mismatch()),
    }
}

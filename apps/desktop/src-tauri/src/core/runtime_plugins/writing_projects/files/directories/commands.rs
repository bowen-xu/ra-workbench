use super::super::super::failure;
use super::super::{FileRequest, FileResult, operate_files};
use super::DirectoryCall;
use crate::{
    runtime_plugins::{
        RuntimePluginCatalog,
        native::{NativePreviewService, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    writing_directory::DirectoryRequest,
    writing_project::{Manuscript, ManuscriptDirectory},
};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn list_runtime_manuscript_directories(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Vec<ManuscriptDirectory>> {
    match operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Directory {
            call: DirectoryCall::List,
        },
    )
    .await?
    {
        FileResult::Directories(values) => Ok(values),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "目录列表结果种类不匹配。",
            &package_key,
        )),
    }
}
#[tauri::command]
pub(crate) async fn create_runtime_manuscript_directory(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    relative_path: String,
) -> NativeResult<ManuscriptDirectory> {
    match operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Directory {
            call: DirectoryCall::Create { relative_path },
        },
    )
    .await?
    {
        FileResult::Directory(value) => Ok(value),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "目录创建结果种类不匹配。",
            &package_key,
        )),
    }
}
#[tauri::command]
pub(crate) async fn rename_runtime_manuscript_directory(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    from_path: String,
    to_path: String,
) -> NativeResult<Manuscript> {
    let request = DirectoryRequest::Rename {
        manuscript_id: manuscript_id.trim().into(),
        from_path: from_path.trim().into(),
        to_path: to_path.trim().into(),
    };
    match operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Directory {
            call: DirectoryCall::Change { request },
        },
    )
    .await?
    {
        FileResult::Project(value) => Ok(value),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "目录改名结果种类不匹配。",
            &package_key,
        )),
    }
}
#[tauri::command]
pub(crate) async fn delete_runtime_manuscript_directory(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    relative_path: String,
) -> NativeResult<()> {
    let request = DirectoryRequest::Delete {
        manuscript_id: manuscript_id.trim().into(),
        relative_path: relative_path.trim().into(),
    };
    match operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Directory {
            call: DirectoryCall::Change { request },
        },
    )
    .await?
    {
        FileResult::Deleted => Ok(()),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "目录删除结果种类不匹配。",
            &package_key,
        )),
    }
}

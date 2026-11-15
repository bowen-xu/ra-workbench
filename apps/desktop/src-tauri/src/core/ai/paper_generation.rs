//! Paper-owned conversation admission and source preparation for both reading stages.
use super::error::AiError;
use crate::storage::AppStore;

/// Prepare selected files before admitting a new conversation turn. No DB lock crosses await.
pub(super) async fn prepare_sources(
    store: &AppStore,
    paper: &str,
    files: &[String],
) -> Result<(), AiError> {
    if files.is_empty() || files.len() > 8 {
        return Err(AiError::InvalidInput("请选择 1–8 个来源文件".into()));
    }
    let preparation = store.docling.get().ok_or_else(|| {
        AiError::InvalidInput("文档解析服务尚未初始化，无法启动文献生成任务".into())
    })?;
    for file in files {
        let run = preparation
            .ensure(store, paper, file)
            .await
            .map_err(|error| AiError::InvalidInput(format!("来源文件 {file} 准备失败：{error}")))?;
        if !matches!(
            run.summary.parse_status.as_str(),
            "success" | "partial_success"
        ) || run
            .json_content
            .as_ref()
            .is_none_or(serde_json::Value::is_null)
        {
            return Err(AiError::InvalidInput(format!(
                "来源文件 {file} 解析失败：{}",
                run.summary
                    .error_message
                    .as_deref()
                    .unwrap_or("没有可用的结构化内容")
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "paper_generation/tests.rs"]
mod tests;

use thiserror::Error;
#[derive(Debug, Error)]
pub(crate) enum VectorError {
    #[error("语义向量索引输入无效：{0}")]
    InvalidInput(&'static str),
    #[error("必需的语义引擎动态库无法加载：{0}")]
    EngineLoad(String),
    #[error("语义引擎 ABI 不兼容")]
    EngineAbiMismatch,
    #[error("语义引擎协议响应无效：{0}")]
    EngineProtocol(String),
    #[error("语义引擎操作失败：{code}")]
    EngineOperation { code: String },
    #[error("语义向量索引后台任务不可用")]
    WorkerUnavailable,
}

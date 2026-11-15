//! Preserve fixed protocol/process causes without accepting arbitrary child or IO text.
pub(super) fn safe_message(message: &str) -> Option<&str> {
    match message {
        "原生插件连接已失效，旧请求不会重放。"
        | "原生插件未在期限内回复；此请求不会自动重放。"
        | "原生输入未确认清理；本次不能报告完成。"
        | "原生插件在接收请求前关闭输入。"
        | "原生插件在回复前退出。"
        | "原生插件超出诊断输出预算；原始输出未保存。"
        | "原生插件输出为空帧或超出64MiB帧预算。"
        | "原生插件发送了未关联的额外回复。"
        | "原生插件未确认初始化。"
        | "原生插件回复协议不匹配。"
        | "原生插件未返回目录结果。"
        | "原生目录结果种类不匹配。"
        | "目录结果拥有者不可用。"
        | "目录结果读取任务未完成。"
        | "目录结果与本次阶段不匹配。"
        | "旧原生实例或输入未确认收尾；请重启应用后再试。"
        | "应用正在关闭原生插件。"
        | "应用正在关闭结构化校验。"
        | "原生插件包核对任务未完成。"
        | "原生插件资源准备任务未完成。"
        | "原生插件准备／启动／握手超过10秒；此请求不会自动重放。"
        | "更换活动原生插件包需要重新启动应用。"
        | "原生插件实例不可用。"
        | "上次文献决定已取消，旧连接不可复用。"
        | "此平台尚未提供文献宿主。"
        | "结构化校验结果种类不匹配。"
        | "结构化校验返回无效规范化 JSON。"
        | "结构化校验与原输入不匹配。"
        | "结构化校验缺少已完成请求身份。"
        | "结构化校验的文献提供者不可用。"
        | "受控目录存在多个文献包，结构化校验未选择提供者。"
        | "所选文献包未声明结构化校验或其冻结来源读取能力。"
        | "所选文献包未声明未校验范围的纯选择能力。"
        | "未校验字段选择结果种类不匹配。"
        | "未校验字段选择与原输入不匹配。"
        | "未校验字段选择超出本次字段范围。"
        | "结构化 HBS 操作未确认结算，禁止自动重放。"
        | "所选文献包未声明论文上下文的纯准备能力。"
        | "所选文献包未声明显式冻结来源读取方法。"
        | "应用正在关闭论文来源准备。"
        | "论文上下文的纯准备未完成。"
        | "论文上下文纯结果与本次阶段或输入不匹配。"
        | "论文来源读取未确认结算；请核对原调用与快照，不要自动重复写入。"
        | "当前会话未绑定可用的文献来源插件。"
        | "native plugin frame exceeds its byte budget"
        | "invalid native plugin JSON frame"
        | "invalid literature plugin envelope"
        | "literature plugin response does not match its request"
        | "invalid request-bound literature snapshot reference"
        | "invalid literature trace decision"
        | "invalid process specification"
        | "process resources could not be released safely"
        | "process backend is not supported on this platform" => Some(message),
        _ if process_io(message) => Some(message),
        _ if paper_context_payload(message) => Some(message),
        _ => None,
    }
}
fn paper_context_payload(message: &str) -> bool {
    let Some((kind, offset)) = message
        .strip_prefix("paper-context payload invalid (kind=")
        .and_then(|value| value.strip_suffix(')'))
        .and_then(|value| value.split_once(", offset="))
    else {
        return false;
    };
    matches!(
        kind,
        "Base64" | "Encode" | "Syntax" | "Semantic" | "Depth" | "Truncated" | "Trailing"
    ) && (offset == "None"
        || offset
            .strip_prefix("Some(")
            .and_then(|value| value.strip_suffix(')'))
            .is_some_and(|value| {
                !value.is_empty()
                    && value.bytes().all(|byte| byte.is_ascii_digit())
                    && value.parse::<usize>().is_ok()
            }))
}
fn error_kind(value: &str) -> bool {
    matches!(
        value,
        "NotFound"
            | "PermissionDenied"
            | "ConnectionRefused"
            | "ConnectionReset"
            | "HostUnreachable"
            | "NetworkUnreachable"
            | "ConnectionAborted"
            | "NotConnected"
            | "AddrInUse"
            | "AddrNotAvailable"
            | "NetworkDown"
            | "BrokenPipe"
            | "AlreadyExists"
            | "WouldBlock"
            | "NotADirectory"
            | "IsADirectory"
            | "DirectoryNotEmpty"
            | "ReadOnlyFilesystem"
            | "FilesystemLoop"
            | "StaleNetworkFileHandle"
            | "InvalidInput"
            | "InvalidData"
            | "TimedOut"
            | "WriteZero"
            | "StorageFull"
            | "NotSeekable"
            | "QuotaExceeded"
            | "FileTooLarge"
            | "ResourceBusy"
            | "ExecutableFileBusy"
            | "Deadlock"
            | "CrossesDevices"
            | "TooManyLinks"
            | "InvalidFilename"
            | "ArgumentListTooLong"
            | "Interrupted"
            | "Unsupported"
            | "UnexpectedEof"
            | "OutOfMemory"
            | "InProgress"
            | "Other"
            | "Uncategorized"
    )
}
fn process_io(message: &str) -> bool {
    if let Some((initial, recovery)) = message.split_once("; termination recovery failed (") {
        return process_io_single(initial)
            && recovery.strip_suffix(')').is_some_and(process_io_single);
    }
    process_io_single(message)
}
fn process_io_single(message: &str) -> bool {
    let Some(inner) = message
        .strip_prefix("process I/O failed (")
        .and_then(|text| text.strip_suffix(')'))
    else {
        return false;
    };
    if error_kind(inner) {
        return true;
    }
    let Some((operation, rest)) = inner
        .strip_prefix("operation=")
        .and_then(|value| value.split_once(", kind="))
    else {
        return false;
    };
    if !matches!(
        operation,
        "group_interrupt"
            | "group_terminate"
            | "child_start_kill"
            | "child_wait"
            | "group_cleanup_after_wait"
    ) {
        return false;
    }
    let Some((kind, errno)) = rest.split_once(", raw_os_error=") else {
        return false;
    };
    error_kind(kind)
        && (errno == "None"
            || errno
                .strip_prefix("Some(")
                .and_then(|value| value.strip_suffix(')'))
                .is_some_and(|value| value.parse::<i32>().is_ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_codec_retains_only_closed_kinds_and_numeric_offsets() {
        for kind in [
            "Base64",
            "Encode",
            "Syntax",
            "Semantic",
            "Depth",
            "Truncated",
            "Trailing",
        ] {
            for offset in ["None", "Some(0)", "Some(184)"] {
                let message =
                    format!("paper-context payload invalid (kind={kind}, offset={offset})");
                assert_eq!(safe_message(&message), Some(message.as_str()));
            }
        }
        for message in [
            "paper-context payload invalid (kind=PRIVATE_BODY, offset=None)",
            "paper-context payload invalid (kind=Syntax, offset=Some(/private/path))",
            "paper-context payload invalid (kind=Semantic, offset=None): private source",
        ] {
            assert_eq!(safe_message(message), None);
        }
    }
    #[test]
    fn process_failure_accepts_only_the_original_single_or_two_attempt_shape() {
        let single = "process I/O failed (operation=group_terminate, kind=PermissionDenied, raw_os_error=Some(1))";
        let simple = "process I/O failed (BrokenPipe)";
        assert_eq!(safe_message(single), Some(single));
        assert_eq!(safe_message(simple), Some(simple));
        let recovery = format!("{single}; termination recovery failed ({simple})");
        assert_eq!(safe_message(&recovery), Some(recovery.as_str()));
        let nested = format!("{single}; termination recovery failed ({recovery})");
        assert_eq!(safe_message(&nested), None);
        assert_eq!(
            safe_message("process I/O failed (PRIVATE_BODY /private/path)"),
            None
        );
    }
}

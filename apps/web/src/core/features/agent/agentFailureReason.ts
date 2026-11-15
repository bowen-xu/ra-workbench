/** Translate safe, bounded runtime diagnostics without guessing their cause. */
export function agentFailureReason(error: string): string {
  if (error.startsWith("tool_outcome_unknown:")) return "工具结果尚未确认，已停止继续执行；请先核对原操作，不要重新发送以重做。";
  if (error.startsWith("tool request rejected: ")) return "工具请求在执行前校验时被拒绝；可复制诊断信息查看调用线索。";
  const reasons: Readonly<Record<string, string>> = {
    "model authentication required": "模型服务认证失败：凭据无效或没有访问权限。",
    "model capability unavailable": "本次请求涉及当前执行路径尚未支持的能力；运行记录未提供具体项目。",
    "unsupported model response type": "模型响应包含当前应用尚未支持的类型，需要更新响应适配。",
    "invalid model request": "模型请求参数或上下文无效（invalid model request）。",
    "model provider rejected the request": "模型服务拒绝了请求，未提供更具体的原因。",
    "model rate limited": "模型服务限制了请求频率。",
    "model context window exceeded": "对话内容超过了模型的上下文容量。",
    "model usage limit reached": "模型服务的使用额度已耗尽。",
    "model server overloaded": "模型服务器负载过高，无法处理请求。",
    "model server internal error": "模型服务器发生内部错误。",
    "model request timed out": "等待模型服务响应超时。",
    "model connection failed": "无法建立与模型服务的连接。",
    "model response read failed": "模型响应传输中断，未能接收完整内容。",
    "model temporarily unavailable": "模型服务暂时不可用；运行记录未提供更具体的原因。",
    "invalid model response stream": "模型返回的响应格式无效，无法解析。",
    "model response ended before completion": "模型响应流提前结束，未收到完成标记。",
    "model output was truncated": "模型输出达到长度限制，回复被截断。",
    "model output was filtered": "模型服务过滤了输出内容。",
    "model response exceeded its budget": "模型响应超过了本次请求的输出上限。",
    "run budget exhausted": "本轮达到模型调用、工具调用或输出上限，无法继续。",
    "tool request rejected": "工具请求未通过当前权限或调用规则校验。",
    "tool execution unavailable": "工具当前不可用。",
    "tool execution outcome unknown": "工具结果尚未确认，已停止继续执行；请先核对原操作，不要重新发送以重做。",
    "run event persistence failed": "运行记录保存失败。",
    "模拟网络连接中断": "模拟故障：与模型服务的网络连接中断。",
  };
  const status = /^unexpected model HTTP status (\d{3})$/u.exec(error)?.[1];
  const authentication = /^model authentication rejected with HTTP (401|403)$/u.exec(error)?.[1];
  if (authentication) return `模型服务拒绝了认证（HTTP ${authentication}），请核对登录状态、凭据与模型访问权限。`;
  if (error === "model rate limited with HTTP 429") return "模型服务限制了请求频率（HTTP 429）。";
  return reasons[error] ?? (status ? `模型服务返回 HTTP ${status}${status === "503" ? "（服务不可用）" : status === "502" ? "（网关错误）" : status === "504" ? "（网关超时）" : ""}。` : error);
}

/** The native diagnostic is a bounded, sanitized protocol value, never an HTTP body. */
export function agentFailureCode(error: string): string {
  if (error.startsWith("tool_outcome_unknown:")) return "tool_outcome_unknown";
  if (error.startsWith("tool request rejected: ")) return "tool_request_rejected";
  return /^(?:model|unexpected model|unsupported model|invalid model|run|tool) [a-zA-Z0-9 ]{1,100}$/u.test(error)
    ? error.toLowerCase().replaceAll(" ", "_") : "agent_failure";
}

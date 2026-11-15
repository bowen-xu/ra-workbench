/** Converts low-level Agent failures into a user-actionable next step. */
export function agentFailureHint(content: string): string | undefined {
  if (/patch[_ ]conflict/iu.test(content)) return "补丁上下文与目标内容不匹配，修改未应用。请核对补丁格式，并重新读取目标内容后重试。";
  if (/tool[_ ]rejected|tool request was rejected during execution/iu.test(content)) return "工具请求被当前执行策略拒绝。请检查项目写入权限、命令执行许可或受保护路径设置后重试。";
  if (/tool[_ ]unavailable|tool is currently unavailable/iu.test(content)) return "工具当前不可用。请稍后重试；如果持续失败，请检查 Agent 配置和工具服务状态。";
  if (/permission denied|access denied|operation not permitted/iu.test(content)) return "工具没有足够的文件或命令权限。请确认当前项目已授权写入，并避开受保护目录。";
  if (/invalid model request/iu.test(content)) return "模型请求参数不受当前提供方支持。请重新选择模型或推理强度后重试。";
  return undefined;
}

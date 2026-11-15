/** Stable UI scene names. Backends implement these by changing fixture ports. */
export const agentCanvasScenarios = [
  ["single", "单条文本消息"], ["running", "运行中"], ["internalRunning", "内部对话运行中"],
  ["completed", "已完成"], ["canceled", "已停止"], ["failed", "工具失败"],
  ["longMessage", "长消息和引用复制"], ["toolHistory", "工具历史 · 运行中"], ["toolHistoryDone", "工具历史 · 已结束（含失败）"],
  ["resources", "技能、工具和脚本"], ["evidence", "批注引用"], ["invalidSkill", "技能版本失效"],
  ["multiple", "多次返回"], ["markdown", "Markdown 消息"],
  ["longHistory", "长历史 · 60 轮"],
  ["submissionPreparing", "正在核对请求"], ["submissionCheckingFailed", "请求核对后失败"],
  ["submissionPrepareFailed", "发送准备失败"], ["submissionStartFailed", "发送启动失败"],
  ["metadataChecking", "正在核对会话状态"], ["metadataFailed", "会话核对失败后重试"],
  ["childCreationUnknown", "子任务创建待核对"],
  ["childCleanupUnknown", "子任务清理待核对"],
] as const;
export type AgentCanvasScenario = typeof agentCanvasScenarios[number][0];
export function isAgentCanvasScenario(value: string): value is AgentCanvasScenario { return agentCanvasScenarios.some(([id]) => id === value); }

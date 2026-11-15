/** Synthetic provider text shared by both fixture adapters. */
export const writingDemoSingleMessage = "这是模拟的单条文本回复。本轮只返回这一条消息，不调用工具，也不修改稿件。";
export const writingDemoResponseMessages = [
  "已检查稿件，先更新 keywords，再补充摘要。",
  "已完成第一次修改：更新 keywords。继续补充摘要。",
  "已完成第二次修改：补充摘要。现在检查两次修改的结果。",
  "检查通过，两次文件修改均已保留。",
] as const;

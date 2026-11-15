export class WritingDispatchAdmissionClosed extends Error {
  readonly code = "WRITING_DISPATCH_ADMISSION_CLOSED";
  constructor(readonly step: "prepare" | "start" | "resume" | "rebase",
    readonly conversationId: string, readonly manuscriptId: string, readonly requestId: string) {
    super("写作插件实例已停止接纳新操作");
  }
}

export function writingDispatchAdmissionMessage(cause: unknown): string | null {
  if (!(cause instanceof WritingDispatchAdmissionClosed)) return null;
  return `写作插件已停止接纳，此步骤尚未派发，原请求和草稿已保留。 [code=${cause.code}；step=${cause.step}；C=${cause.conversationId}；M=${cause.manuscriptId}；R=${cause.requestId}]`;
}

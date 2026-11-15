import type { AgentEvidenceInput } from "@research-assistant/core/agent";
import type { AgentCanvasMessage, AgentCanvasMessageOptions } from "./AgentCanvasMessage";
import { releaseRemovedEvidence } from "./evidenceImageGateway";

export type SelectionAnnotationIntent = "send" | "silent";

/** Add captures a draft; Send keeps the existing composer draft independent. */
export async function submitEvidenceAnnotation({ capture, resources, comment, intent, submit, discard }: {
  readonly capture: () => Promise<readonly AgentEvidenceInput[]>;
  readonly resources: AgentCanvasMessage["resources"];
  readonly comment: string;
  readonly intent?: SelectionAnnotationIntent;
  readonly submit: (text: string, resources?: AgentCanvasMessage["resources"], options?: AgentCanvasMessageOptions) => Promise<void>;
  readonly discard: () => void;
}): Promise<void> {
  if (intent === "send" && !comment.trim()) throw new Error("请输入要询问 AI 的问题。");
  const evidence = (await capture()).map(item => ({ ...item, comment }));
  if (!evidence.length) throw new Error("请先选择内容。");
  try {
    if (intent === "send") await submit(comment, resources, { evidence });
    else await submit("", resources, { mode: "draft", evidence, ...(intent === "silent" ? { reveal: false as const } : {}) });
    discard();
  } catch (reason) { releaseRemovedEvidence(evidence); throw reason; }
}

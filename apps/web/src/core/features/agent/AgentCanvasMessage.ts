/** A reader action submits through chat without taking its composer draft. */
export interface AgentCanvasMessage {
  readonly text: string;
  readonly options?: AgentCanvasMessageOptions;
  readonly resources?: readonly import("@research-assistant/core/agent").AgentContextResource[];
  readonly resolve: () => void;
  readonly reject: (reason: unknown) => void;
}
export interface AgentCanvasMessageOptions {
  readonly evidence?: readonly import("@research-assistant/core/agent").AgentEvidenceInput[];
  readonly mode?: "draft";
  /** Draft capture may preserve the current window visibility. */
  readonly reveal?: false;
}

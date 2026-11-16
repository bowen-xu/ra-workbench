import { AgentModelControl } from "../../../../../../apps/web/src/core/public/agentPresentation/AgentModelControl";
import type { AgentModelControlProps } from "../../../../../../apps/web/src/core/public/agentPresentation/AgentModelControl";

/**
 * The production Agent reader uses the same model selector as the writing and
 * note hosts. The vision kind is the only reader-specific policy; session
 * selection remains controlled by the reader panel.
 */
export function ReaderAgentModelControl(
  props: Omit<AgentModelControlProps, "route" | "label" | "modelKind">,
): React.JSX.Element {
  return <AgentModelControl {...props} modelKind="vision" route="paper_reading_chat" label="选择 AI 文献阅读模型" />;
}

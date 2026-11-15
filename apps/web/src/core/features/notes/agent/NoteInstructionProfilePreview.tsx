import type { AgentSubmissionInput } from "@research-assistant/core/agent";
import { AgentInstructionProfilePreview } from "../../agent/AgentInstructionProfilePreview";

export function NoteInstructionProfilePreview({ input }: { readonly input: AgentSubmissionInput }): React.JSX.Element {
  return <AgentInstructionProfilePreview input={input} subject="笔记" />;
}

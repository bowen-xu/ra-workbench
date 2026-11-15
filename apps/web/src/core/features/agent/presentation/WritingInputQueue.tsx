import { AgentInputQueue, type AgentInputQueueProps } from "../../../public/agentSubmission/AgentInputQueue";
export type WritingInputQueueProps = Omit<AgentInputQueueProps, "className" | "explanation">;
export function WritingInputQueue(props: WritingInputQueueProps): React.JSX.Element {
  return <AgentInputQueue {...props} className="writing-agent-input-queue" explanation="队列会在当前回合结束或宿主空闲时自动发送；展开后可编辑、移除或调整顺序。"/>;
}

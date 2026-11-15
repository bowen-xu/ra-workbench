/** Same Core presentation producers, kept separate from the Canvas entry. */
export {WritingAgentPanel} from "./agentPanel";
export type {WritingAgentSubmitOptions, WritingAgentMessage} from "./agentPanel";
export type {AgentDomainSessionProps} from "./agentConversationAdapters";
export {WritingAgentTaskStack} from "./agentTaskStack";
export type {WritingAgentTaskStackProps, WritingAgentGoal, WritingAgentGoalState} from "./agentTaskStack";
export {AgentChangeContribution} from "./agentChanges";
export {AgentTransientFailure} from "./agentPresentation/AgentFailureMessage";
export {AgentRecoveryMessage} from "./agentPresentation/AgentRecoveryMessage";
export {readMessageInspection} from "./agentPresentation/messageInspectionGateway";
export type {ReadMessageInspection} from "./agentPresentation/messageInspectionGateway";
export {useAgentChildTasks} from "./agentPresentation/useAgentChildTasks";
export {useAgentHostLifecycle, useAgentMountedGuard} from "./agentPresentation/useAgentMountedGuard";

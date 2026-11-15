import type { ReactNode } from "react";
import type { LlmModelSelection } from "@research-assistant/core/ai";
import type { AgentUserInput, AgentResourceScope, AgentChildTaskSummary, AgentTaskKind } from "@research-assistant/core/agent";
import type { ReadMessageData } from "../../../public/agentPresentation/AgentMessageData";
import type { AgentComposerDraftAccess } from "../../../public/agentSubmission/useAgentComposerInput";
import type { AgentCommandGateway } from "../../../public/agentSubmission/agentCommandGateway";
import type { AgentCommandExecution } from "../../../public/agentSubmission/useAgentCommandExecution";
import type { AgentModelGateway } from "../../../public/agentPresentation/AgentModelControl";
import type { AgentResourceGateways } from "../../../public/agentContext/agentResourceGateways";
import type { AgentMessageSearchTarget } from "../../../public/agentPresentation/useMessageSearchTarget";
import type { WritingAgentContextScope, WritingAgentContextUsage } from "./WritingAgentContextControl";
import type { WritingAgentMessage } from "./WritingAgentMessage";
import type { WritingAgentSizePreset } from "./useWritingAgentSize";

export interface WritingAgentSubmitOptions {
  readonly userInput?: readonly AgentUserInput[];
  readonly contextScope: WritingAgentContextScope;
  readonly goalMode?: boolean;
  readonly modelSelection?: LlmModelSelection;
  readonly includeConceptRetrieval?: boolean;
}

export interface WritingAgentPanelProps {
  readonly approvalConversationId?: string;
  /** Independent hosts own context selection outside the writing composer. */
  readonly independentContext?: boolean;
  readonly headerControls?: ReactNode;
  readonly showLauncher?: boolean | undefined;
  /** Embedded previews may keep the panel open while their outer controls are used. */
  readonly dismissOnOutsideClick?: boolean | undefined;
  readonly inputDisabled?: boolean;
  /** A visible failure replaces the temporary preparation indicator. */
  readonly suppressPreparing?: boolean;
  readonly isSubmitting?: boolean;
  /** Queue preparation is shown in the composer without replacing its draft. */
  readonly queuePreparation?: {
    readonly status: "sending" | "recovery";
    readonly onRecover?: () => void;
    readonly recoveryDisabled?: boolean;
  } | undefined;
  /** A provisional user message is visible while its send is resolved. */
  readonly hasPendingSubmission?: boolean;
  readonly submittingText?: string;
  /** Frozen file changes belong to their completed turn, before its footer. */
  readonly turnAttachment?: (turnId: string) => ReactNode;
  readonly turnFeedback?: (turnId: string) => ReactNode;
  readonly turnAttachmentPaths?: (turnId: string) => ReadonlySet<string>;
  /** Optional content attached to a mock or real assistant message. Kept in the
   * panel so previews and production transcripts use the same message shell. */
  readonly readMessageData?: ReadMessageData;
  readonly messageExtras?: (message: WritingAgentMessage) => ReactNode;
  readonly messageActions?: (messageId: string) => ReactNode;
  readonly historyActions?: ReactNode;
  readonly historyHeader?: ReactNode;
  readonly searchTarget?: AgentMessageSearchTarget | null | undefined;
  readonly historyReady?: boolean;
  readonly historyError?: boolean | undefined;
  readonly draft?: string;
  readonly onDraftChange?: (value: string) => void;
  readonly submissionControls?: ReactNode | ((options: WritingAgentSubmitOptions) => ReactNode);
  readonly contextControls?: ReactNode;
  readonly composerAccessory?: ReactNode | ((options: WritingAgentSubmitOptions) => ReactNode);
  readonly draftResources?: AgentComposerDraftAccess;
  readonly resourceScope?: AgentResourceScope;
  readonly onQueue?: (text: string, userInput?: readonly AgentUserInput[]) => void;
  readonly queueDraftEditing?: boolean;
  readonly queueDisabled?: boolean;
  readonly activePath?: string;
  readonly disabledReason?: string;
  readonly contextUsage?: WritingAgentContextUsage;
  readonly modelGateway?: AgentModelGateway;
  readonly resourceGateways?: AgentResourceGateways;
  readonly commandGateway?: AgentCommandGateway;
  readonly commandExecution?: AgentCommandExecution;
  readonly onQueueCommand?: (text: string, originalDraft: string) => void;
  readonly isRunning?: boolean;
  readonly activeTaskKind?: AgentTaskKind | undefined;
  /** Durable start time of the active turn, used to keep elapsed time across remounts. */
  readonly thinkingStartedAtMs?: number;
  readonly conversationTitle?: string;
  readonly conversationRename?: import("../../../public/agentConversations/ConversationHeaderTitle").ConversationTitleRenameAccess;
  readonly messages?: readonly WritingAgentMessage[];
  readonly childTasks?: readonly AgentChildTaskSummary[];
  readonly onOpenChildTask?: (conversationId: string) => void;
  readonly onReconcileChildTask?: (conversationId: string) => void;
  readonly onResumeChildTask?: (conversationId: string) => void;
  readonly onCancel?: () => void;
  /** Request cancellation of the currently pending send, without assuming it succeeded. */
  readonly onCancelSending?: () => void;
  readonly cancelSendingPending?: boolean;
  readonly onSteer?: (text: string, userInput?: readonly AgentUserInput[]) => void;
  readonly steeringPending?: boolean;
  readonly onOpenChange?: (open: boolean) => void;
  readonly onModelSelectionChange?: (selection: LlmModelSelection | undefined) => void;
  readonly onSubmit?: (message: string, options: WritingAgentSubmitOptions) => void | false;
  /** Host prepares commands with the same selected model and resource scope. */
  readonly onSubmitCommand?: (command: string, options: WritingAgentSubmitOptions) => void;
  readonly open?: boolean;
  /** Optional initial size used by embedded previews; regular callers keep the persisted size. */
  readonly defaultSize?: WritingAgentSizePreset;
}

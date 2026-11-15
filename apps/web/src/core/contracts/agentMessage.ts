import type { AgentTaskKind, AgentTurnOutcome } from "@research-assistant/core/agent";

export interface ChildCreationRecoveryIdentity {
  readonly conversationId: string; readonly turnId: string; readonly callId: string;
  readonly childId: string; readonly operationId: string; readonly watermark: number;
  readonly submissionRequestId?: string;
}

export interface AgentMessage {
  readonly conversationId?: string;
  readonly userInput?: readonly import("@research-assistant/core/agent").AgentUserInput[];
  readonly content: string;
  /** Original journal projection, separate from display text and suffixes. */
  readonly rawData?: unknown;
  /** ISO timestamp used for the message metadata row. */
  readonly createdAt?: string;
  readonly failure?: boolean;
  /** Allowlisted runtime metadata for one-click failure diagnostics. */
  readonly failureDiagnosticText?: string;
  readonly childCreationRecovery?: readonly (ChildCreationRecoveryIdentity & {
    readonly failureStage?: "creation" | "cleanup";
  })[];
  /** Receipt identities actually represented by this recovery message. */
  readonly unconfirmedToolReceiptIds?: readonly string[];
  readonly id: string;
  /** Stable presentation identity across retry samples; journal ID stays exact. */
  readonly renderKey?: string;
  readonly phase?: "commentary" | "reasoning" | "finalAnswer";
  readonly turnId?: string;
  readonly taskKind?: AgentTaskKind;
  /** Item completion alone does not end a response. */
  readonly turnFinished?: boolean;
  readonly turnOutcome?: AgentTurnOutcome;
  readonly turnStartedAtMs?: number;
  readonly turnFinishedAtMs?: number;
  readonly reconnecting?:boolean;
  readonly reconnectAttempt?: number | undefined;
  readonly reconnectHistory?: readonly { readonly attemptId: string; readonly reason: string; readonly failedSeq?: number; readonly attemptIndex?: number }[];
  /** Durable sequence of the item's first event, used only for timeline placement. */
  readonly createdSeq?: number;
  readonly attemptIndex?: number;
  readonly kind?: "tool-call" | "tool-result" | "reasoning-summary" | "context-compaction" | "reconnect-anchor";
  /** The durable operation exists before its candidate window is installed. */
  readonly compaction?: { readonly operationId: string; readonly windowId: string | null };
  /** Public summary parts only. Native continuation data is never a display source. */
  readonly reasoningSummaryParts?: readonly import("@research-assistant/core/agent").AgentReasoningSummaryTextPart[];
  readonly toolError?: boolean;
  readonly toolName?: string;
  /** Links a tool result to its call while the turn is still running. */
  readonly toolCallId?: string;
  readonly role: "assistant" | "user";
  readonly streaming?: boolean;
  readonly interrupted?: boolean;
}

/** Legacy source name retained for consumers while the shared presentation migrates. */
export type WritingAgentMessage = AgentMessage;

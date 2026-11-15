import { Fragment, memo, useMemo, useRef, type ReactNode } from "react";
import type { AgentTaskKind } from "@research-assistant/core/agent";
import { AgentMessageData, type ReadMessageData } from "../../../public/agentPresentation/AgentMessageData";
import { AgentMessageCopyButton } from "../../../public/agentPresentation/AgentMessageCopyButton";
import { AgentFailureMessage } from "../../../public/agentPresentation/AgentFailureMessage";
import { AgentChildCreationRecovery } from "../../../public/agentPresentation/AgentChildCreationRecovery";
import { AgentUserMessage, AgentUserMessageEvidence } from "../../../public/agentPresentation/AgentUserMessage";
import { AgentUserMessageScope } from "../../../public/agentPresentation/AgentUserMessageScope";
import { AgentTextChangeCard, textPatchTargets } from "../../../public/agentPresentation/AgentTextChangeCard";
import {AgentTurnChangeSummary} from "../../../public/agentChanges";
import type { AgentResourceGateways } from "../../../public/agentContext/agentResourceGateways";
import { AgentCompactionActivity, type AgentCompactionRecord } from "../../../public/agentPresentation/AgentCompactionActivity";
import { useDeveloperMode } from "../../../public/settings";
import { WritingAgentActivityGroup, type WritingAgentActivityProps } from "./WritingAgentActivity";
import { WritingAgentMarkdown } from "./WritingAgentMarkdown";
import { assistantMetadataIds } from "./WritingAgentMessageMetadata";
import type { WritingAgentMessage } from "./WritingAgentMessage";
import { WritingAgentTurn, type WritingAgentTurnPart } from "./WritingAgentTurn";
import { AgentReasoningSummary } from "./AgentReasoningSummary";
import { AgentReconnectActivity } from "./AgentReconnectActivity";

import {useTranscriptWindow} from "./useTranscriptWindow";
import {TranscriptDisclosureState} from "./TranscriptDisclosureState";
import type { AgentMessageSearchTarget } from "../../../public/agentPresentation/useMessageSearchTarget";
import { MarkdownRenderScope } from "../markdown/MarkdownRenderScope";

interface TranscriptProps {
  readonly searchTarget?: AgentMessageSearchTarget | null | undefined;
  readonly resourceGateways?: AgentResourceGateways | undefined;
  readonly messages: readonly WritingAgentMessage[];
  readonly readMessageData?: ReadMessageData | undefined;
  readonly isRunning: boolean;
  /** A durable task may be active before its first item (for example PreCompact). */
  readonly activeTaskKind?: AgentTaskKind | undefined;
  readonly thinkingStartedAtMs?: number | undefined;
  readonly turnAttachment?: ((turnId: string) => ReactNode) | undefined;
  readonly turnFeedback?: ((turnId: string) => ReactNode) | undefined;
  /** Only file paths confirmed in the displayed attachment suppress duplicate receipts. */
  readonly turnAttachmentPaths?: ((turnId: string) => ReadonlySet<string>) | undefined;
  readonly messageExtras?: ((message: WritingAgentMessage) => ReactNode) | undefined;
  readonly messageActions?: ((messageId: string) => ReactNode) | undefined;
  readonly matchedId?: string | undefined;
  readonly searchMessageId?: string | undefined;
  readonly commandActivities?: readonly AgentCompactionRecord[] | undefined;
}
interface Turn {
  readonly id: string;
  readonly messages: WritingAgentMessage[];
}
type TimelineEntry = { readonly kind: "message"; readonly message: WritingAgentMessage }
  | { readonly kind: "reasoning-summary"; readonly message: WritingAgentMessage }
  | { readonly kind: "context-compaction"; readonly message: WritingAgentMessage }
  | { readonly kind: "reconnect"; readonly retries: readonly RetryEntry[]; readonly reconnecting: boolean; readonly attempt: number | undefined }
  | { readonly kind: "activity-group"; readonly items: ToolMessage[] };
type RetryEntry = NonNullable<WritingAgentMessage["reconnectHistory"]>[number];
type ToolMessage = WritingAgentMessage & { readonly kind: "tool-call" | "tool-result" };
function isToolMessage(message: WritingAgentMessage): message is ToolMessage {
  return message.kind === "tool-call" || message.kind === "tool-result";
}

function turns(messages: readonly WritingAgentMessage[]): Turn[] {
  const result: Turn[] = [];
  for (const message of messages) {
    const previous = result.at(-1);
    // Durable turn identity keeps Steer inside its original turn. Legacy
    // messages use user boundaries, matching the existing footer projection.
    const id = message.turnId ?? (!previous || message.role === "user" ? `legacy:${message.id}` : previous.id);
    if (previous?.id === id) previous.messages.push(message);
    else result.push({ id, messages: [message] });
  }
  return result;
}

function timelineEntries(input: readonly WritingAgentMessage[], retries: readonly RetryEntry[], reconnecting: boolean,
  reconnectAttempt: number | undefined, finalIds: ReadonlySet<string>): TimelineEntry[] {
  const messages = input.filter(message => message.kind !== "reconnect-anchor");
  const retryPositions = new Map<number, RetryEntry[]>();
  for (const retry of retries) {
    let boundary = 0, known = false;
    for (const [index, message] of messages.entries()) {
      if (retry.failedSeq !== undefined && message.createdSeq !== undefined) {
        known = true; if (message.createdSeq <= retry.failedSeq) boundary = index + 1;
      } else if (retry.attemptIndex !== undefined && message.attemptIndex !== undefined) {
        known = true; if (message.attemptIndex <= retry.attemptIndex) boundary = index + 1;
      }
    }
    // Legacy hand-built messages lack sequence evidence; keep the record in
    // the process rather than inventing a precise earlier position.
    if (!known) {
      const failure = messages.findIndex(message => message.failure);
      boundary = failure < 0 ? messages.length : failure;
    }
    const bucket = retryPositions.get(boundary) ?? []; bucket.push(retry); retryPositions.set(boundary, bucket);
  }
  const result: TimelineEntry[] = [];
  const pendingCalls = new Map<string, Extract<TimelineEntry, { kind: "activity-group" }>>();
  function insertRetries(position: number): void {
    const retriesAtPosition = retryPositions.get(position);
    if (retriesAtPosition?.length) {
      // A late tool result must not be folded into a group before the failure.
      pendingCalls.clear();
      result.push({kind: "reconnect", retries: retriesAtPosition, reconnecting: reconnecting && retriesAtPosition.at(-1) === retries.at(-1), attempt: reconnectAttempt});
    }
  }
  for (const [index, message] of messages.entries()) {
    insertRetries(index);
    const previous = result.at(-1);
    // A receipt arriving after an independently rendered final must stay after it.
    if (finalIds.has(message.id)) pendingCalls.clear();
    if (message.kind === "context-compaction") result.push({ kind: "context-compaction", message });
    else if (message.kind === "reasoning-summary") result.push({ kind: "reasoning-summary", message });
    else if (!isToolMessage(message)) result.push({ kind: "message", message });
    else {
      // Model commentary may separate a call from its durable receipt. Keep
      // the receipt with its owning call without moving the commentary.
      const owner = message.kind === "tool-result" && message.toolCallId !== undefined
        ? pendingCalls.get(message.toolCallId) : undefined;
      const group = owner ?? (previous?.kind === "activity-group" ? previous : undefined)
        ?? { kind: "activity-group" as const, items: [] };
      if (!owner && group !== previous) result.push(group);
      group.items.push(message);
      if (message.toolCallId !== undefined) {
        if (message.kind === "tool-call") pendingCalls.set(message.toolCallId, group);
        else if (owner) pendingCalls.delete(message.toolCallId);
      }
    }
  }
  insertRetries(messages.length);
  return result;
}

function activityProps(message: ToolMessage, laterResult?: ToolMessage): WritingAgentActivityProps {
  return {
    content: message.content,
    ...(message.toolCallId === undefined ? {} : {callId:message.toolCallId}),
    ...(message.turnFinished === undefined ? {} : {turnFinished:message.turnFinished}),
    ...(message.interrupted === undefined ? {} : {interrupted:message.interrupted}),
    ...(message.toolError === undefined ? {} : { error: message.toolError }),
    ...(message.toolName === undefined ? {} : { name: message.toolName }),
    ...(message.streaming === undefined ? {} : { streaming: message.streaming }),
    ...(laterResult && !laterResult.streaming ? {laterResult: laterResult.toolError ? "error" as const : laterResult.interrupted ? "interrupted" as const : "success" as const} : {}),
    type: message.kind,
  };
}

function messageTimestamp(message: WritingAgentMessage): Date | undefined {
  const value = message.createdAt === undefined ? undefined : new Date(message.createdAt);
  return value !== undefined && !Number.isNaN(value.getTime()) ? value : undefined;
}

function formatMessageTimestamp(timestamp: Date, now = new Date()): string {
  const sameDay = timestamp.getFullYear() === now.getFullYear()
    && timestamp.getMonth() === now.getMonth() && timestamp.getDate() === now.getDate();
  const timeOptions: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit", hour12: true };
  return new Intl.DateTimeFormat("en-US", sameDay ? timeOptions
    : { ...timeOptions, month: "short", day: "numeric", year: "numeric" }).format(timestamp);
}

export const WritingAgentTranscript = memo(function WritingAgentTranscript({ searchTarget, messages, isRunning, activeTaskKind, thinkingStartedAtMs, turnAttachment, turnFeedback, turnAttachmentPaths,
  messageExtras, messageActions, matchedId, searchMessageId, readMessageData, resourceGateways, commandActivities = [] }: TranscriptProps): React.JSX.Element {
  const metadataIds = assistantMetadataIds(messages, isRunning);
  const latestResponseId = messages.filter(message => message.kind !== "reasoning-summary" && message.kind !== "context-compaction" && message.kind !== "reconnect-anchor").at(-1)?.id;
  const { enabled: developerMode } = useDeveloperMode();
  function renderMetadata(message: WritingAgentMessage): ReactNode {
    const createdAt = messageTimestamp(message);
    const timestamp = createdAt && <time className="writing-agent-message-time" dateTime={createdAt.toISOString()}>{formatMessageTimestamp(createdAt)}</time>;
    return <div className="writing-agent-message-meta">
      {message.role === "user" && timestamp}
      <div className="writing-agent-message-actions" data-agent-message-actions>
        <AgentMessageData messages={[message]} read={readMessageData} className="writing-agent-message-copy" />
        <AgentMessageCopyButton className="writing-agent-message-copy" value={message.content} messageId={message.role === "user" ? message.id : undefined} />{messageActions?.(message.id)}
      </div>
      {message.role === "assistant" && timestamp}
    </div>;
  }
  function render(entry: TimelineEntry, attachment?: ReactNode, scopedMessages = messages, deferredMetadataId?: string): ReactNode {
    if (entry.kind === "reconnect") return <AgentReconnectActivity key={entry.retries[0]?.attemptId} retries={entry.retries}
      reconnecting={entry.reconnecting} attempt={entry.attempt} />;
    const message = entry.kind === "activity-group" ? entry.items[0] : entry.message;
    if (!message) return null;
    if (entry.kind === "context-compaction") return <AgentCompactionActivity key={message.id} message={message} read={readMessageData} />;
    if (entry.kind === "reasoning-summary") return <AgentReasoningSummary key={message.id} message={message} read={readMessageData}
      matched={matchedId === message.id || searchMessageId === message.id} />;
    const streaming = entry.kind === "activity-group" ? entry.items.some((item) => item.streaming) : message.streaming;
    if (entry.kind === "activity-group") {
      const actions = messageActions?.(entry.items.at(-1)?.id ?? message.id);
      return <section aria-label="工具活动" className="writing-agent-activity-record" key={message.id} data-agent-activity-record>
        <div className="writing-agent-message-bubble">
          <WritingAgentActivityGroup disclosureId={message.id} items={entry.items.map(item => {
            const later = item.kind === "tool-call" && item.toolCallId && !entry.items.some(value => value.kind === "tool-result" && value.toolCallId === item.toolCallId)
              ? scopedMessages.find((value): value is ToolMessage => value.kind === "tool-result" && value.toolCallId === item.toolCallId) : undefined;
            return activityProps(item, later);
          })} headerActions={developerMode ? <AgentMessageData messages={entry.items} read={readMessageData} /> : undefined} />
          {messageExtras?.(message)}
        </div>
        {!streaming && actions ? <div className="writing-agent-activity-actions" data-agent-message-actions>{actions}</div> : null}
      </section>;
    }
    return <AgentUserMessageScope key={message.renderKey ?? message.id} message={message} read={readMessageData}><article className={`writing-agent-message ${message.role}${message.failure ? " failure" : ""}${matchedId === message.id ? " search-match" : ""}`}
      key={message.renderKey ?? message.id} data-agent-message-id={message.id} data-deferred-response-owner={message.id === deferredMetadataId ? "true" : undefined} data-latest-response={message.role === "assistant" && message.id === latestResponseId ? "true" : undefined} {...(message.failure ? { role: "alert" } : {})} tabIndex={-1}>
      {message.role === "user" && !message.failure && <AgentUserMessageEvidence message={message} read={readMessageData} />}
      <div className="writing-agent-message-bubble" data-selectable-text>
        {message.childCreationRecovery?.length ? message.childCreationRecovery.map(identity => <AgentChildCreationRecovery key={identity.callId} identity={identity} failureStage={identity.failureStage} diagnosticText={message.failureDiagnosticText} embedded />)
          : message.failure ? <AgentFailureMessage message={message} /> : message.role === "assistant"
          ? <WritingAgentMarkdown value={message.content} {...(message.streaming === undefined ? {} : { streaming: message.streaming })} />
          : <AgentUserMessage message={message} read={readMessageData} gateways={resourceGateways} reveal={matchedId === message.id || searchMessageId === message.id} />}
        {messageExtras?.(message)}
        {attachment}
      </div>
      {!streaming && message.id !== deferredMetadataId && (message.role === "user" || metadataIds.has(message.id)) && renderMetadata(message)}
    </article></AgentUserMessageScope>;
  }

  const rounds = useMemo(() => {
    const rows = turns(messages);
    if (isRunning && (!rows.length || rows.at(-1)?.messages.at(-1)?.turnFinished === true)) rows.push({id: "pending", messages: []});
    return rows;
  }, [messages, isRunning]);
  const windowed = useTranscriptWindow(rounds, searchTarget);
  const disclosures = useRef(new Map<string, boolean>());
  return <MarkdownRenderScope>
    <AgentCompactionActivity records={commandActivities.filter(record => !messages.some(message => message.id === record.afterMessageId))} />
    <div ref={windowed.body} className="agent-transcript-window">
    {windowed.rows.map(({item: turn, index, gap}) => {
      const last = turn.messages.at(-1);
      const latest = index === rounds.length - 1;
      const finished = last?.turnFinished ?? (!isRunning || !latest);
      const running = !finished && latest;
      const outcome = turn.messages.find((message) => message.turnOutcome !== undefined)?.turnOutcome;
      const aborted = !running && (outcome === "canceled" || outcome === "interrupted");
      const prompt = turn.messages[0]?.role === "user" ? turn.messages[0] : undefined;
      const feedback = turn.messages.filter((message) => message.role === "assistant");
      const text = feedback.filter((message) => !message.kind);
      const candidate = text.at(-1);
      const finals = text.filter(message => message.failure || (message.phase === "finalAnswer" && message.content.trim()));
      // Legacy/unknown samples retain their last-message fallback. Known native
      // commentary is never promoted just because the turn has ended.
      if (finals.length === 0 && finished && candidate && candidate.phase === undefined) finals.push(candidate);
      const finalIds = new Set(finals.map(message => message.id));
      const processMessages = turn.messages.filter((message) => message !== prompt && !finalIds.has(message.id));
      const reconnectHistory = turn.messages.find(message => message.reconnectHistory !== undefined)?.reconnectHistory ?? [];
      const reconnecting = running && turn.messages.some(message => message.reconnecting);
      const reconnectAttempt = turn.messages.find(message => message.reconnectAttempt !== undefined)?.reconnectAttempt;
      const manualCompaction = turn.messages.some(message => message.taskKind === "compact")
        || (turn.id === "pending" && activeTaskKind === "compact");
      const compactOnlyProcess = manualCompaction && processMessages.every(message => message.kind === "context-compaction");
      const hasProcess = !compactOnlyProcess && (reconnectHistory.length > 0 || feedback.some((message) => message.kind !== undefined && message.kind !== "reconnect-anchor")
        || text.filter(message => !finalIds.has(message.id)).length > 1
        || text.some(message => !finalIds.has(message.id) && !!message.content.trim()
          && (message.phase === "commentary" || message.phase === "reasoning")));
      const calls = feedback.filter((message) => message.kind === "tool-call");
      const results = feedback.filter((message) => message.kind === "tool-result" && !message.streaming);
      // One summary per owning turn; tool receipts stay in the process journal.
      // Later results for the same path replace earlier target statuses.
      const patchResults = results.filter(message => message.toolName === "research_apply_patch");
      const fileAttachment = finished ? turnAttachment?.(turn.id) : null;
      const coveredFilePaths = fileAttachment ? turnAttachmentPaths?.(turn.id) : undefined;
      const targets = [...new Map(patchResults.flatMap(message => textPatchTargets(message.content)).map(target => [target.path, target])).values()]
        .filter(target => !coveredFilePaths?.has(target.path) || target.status !== "applied" || !!target.structuredResource || !!target.changeId);
      const conversationId = patchResults.find(message => message.conversationId)?.conversationId;
      const attachment = finished ? <AgentTurnChangeSummary key={turn.id}>{fileAttachment}{targets.some(target => target.status !== "failed") && <AgentTextChangeCard
        content={JSON.stringify({ targets })} {...(conversationId ? { conversationId } : {})} />}</AgentTurnChangeSummary> : null;
      const toolPending = calls.some((call) => call.toolCallId === undefined
        ? call.streaming || results.length < calls.length
        : !results.some((result) => result.toolCallId === call.toolCallId));
      const pendingCompaction = turn.messages.some(message => message.kind === "context-compaction"
        && message.compaction?.windowId == null && !message.interrupted);
      const thinking = running && !manualCompaction && !pendingCompaction && finals.length === 0
        && (!hasProcess || (!toolPending && !feedback.some((message) => message.streaming)));
      const startedAtMs = turn.messages.find((message) => message.turnStartedAtMs !== undefined)?.turnStartedAtMs
        ?? (running ? thinkingStartedAtMs : undefined);
      const finishedAtMs = turn.messages.find((message) => message.turnFinishedAtMs !== undefined)?.turnFinishedAtMs;
      const orderedMessages = reconnectHistory.length ? turn.messages.filter(message => message !== prompt) : [...processMessages, ...finals];
      const entries = timelineEntries(orderedMessages, reconnectHistory, reconnecting, reconnectAttempt, finalIds);
      const footerIndex = entries.findIndex(entry => entry.kind === "message" && entry.message.role === "assistant" && metadataIds.has(entry.message.id));
      const feedbackNode = turnFeedback?.(turn.id);
      const footerEntry = footerIndex >= 0 && (footerIndex < entries.length - 1 || feedbackNode) ? entries[footerIndex] : undefined;
      const deferredMessage = footerEntry?.kind === "message" ? footerEntry.message : undefined;
      const timeline: WritingAgentTurnPart[] = [];
      let processNodes: ReactNode[] = [], previousFinal = "start";
      const flushProcess = () => {
        if (processNodes.length) timeline.push({kind: "process", key: `process:${previousFinal}`, content: processNodes});
        processNodes = [];
      };
      for (const entry of entries) {
        if (entry.kind === "message" && finalIds.has(entry.message.id)) {
          flushProcess();
          const key = entry.message.renderKey ?? entry.message.id;
          timeline.push({kind: "final", key, content: render(entry, entry.message.id === finals.at(-1)?.id ? attachment : undefined, turn.messages, deferredMessage?.id)});
          previousFinal = key;
        } else processNodes.push(render(entry, undefined, turn.messages, deferredMessage?.id));
      }
      flushProcess();
      // Final slots retain the projection's retry identity at every keyed level;
      // an item-ID wrapper would remount the correctly keyed inner message.
      return <Fragment key={turn.id}>
        {gap > 0 && <div aria-hidden="true" style={{height: gap}}/>}
        <div data-transcript-row={turn.id}><TranscriptDisclosureState scope={turn.id} states={disclosures.current}>
        {prompt && render({ kind: "message", message: prompt })}
        <WritingAgentTurn initialExpanded={disclosures.current.get(turn.id)} onExpandedChange={open => {disclosures.current.set(turn.id, open);}} turnId={turn.id} running={running} aborted={aborted} failed={outcome === "failed" || turn.messages.some(message => message.failure)} hasProcess={hasProcess} thinking={thinking}
          reconnecting={reconnecting}
          startedAtMs={startedAtMs} finishedAtMs={finishedAtMs} searchMessageId={searchMessageId ?? matchedId}
          timeline={timeline} finalMessage={null} footer={<>{feedbackNode}{deferredMessage && <div className="writing-agent-message assistant" data-deferred-response-footer="true" data-latest-response={deferredMessage.id === latestResponseId ? "true" : undefined}>{renderMetadata(deferredMessage)}</div>}</>}>
        </WritingAgentTurn>
        {finished && finals.length === 0 && attachment}
        <AgentCompactionActivity records={commandActivities.filter(record => turn.messages.some(message => message.id === record.afterMessageId))} />
        </TranscriptDisclosureState></div>
      </Fragment>;
    })}
    {windowed.bottom > 0 && <div aria-hidden="true" style={{height: windowed.bottom}}/>}
    </div>
  </MarkdownRenderScope>;
});

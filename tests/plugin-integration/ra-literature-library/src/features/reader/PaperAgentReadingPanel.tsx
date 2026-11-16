import { createPaperQuestionSubmission } from "./paperQuestionSubmission";
import {SystemDocumentApprovalPanel} from "../../../../../../apps/web/src/core/public/settings";
import { recoverRejectedSteer, pauseRejectedSteerQueue, prioritizeRejectedSteerLease } from "../../../../../../apps/web/src/core/public/agentSubmission/rejectedSteerQueue";
import { readMessageInspection } from "../../../../../../apps/web/src/core/public/agentPresentation/messageInspectionGateway";
import { ControlHint } from "../../../../../../apps/web/src/core/public/workbench";
import { AgentInputQueue } from "../../../../../../apps/web/src/core/public/agentSubmission/AgentInputQueue";
import { AgentCompactionActivity } from "../../../../../../apps/web/src/core/public/agentPresentation/AgentCompactionActivity";
import { AgentComposerTextInput, type AgentComposerInputHandle } from "../../../../../../apps/web/src/core/public/agentSubmission/AgentComposerTextInput";
import type { AgentResourceGateways } from "../../../../../../apps/web/src/core/public/agentContext/agentResourceGateways";
import { AgentInstructionProfilePreview } from "../../../../../../apps/web/src/core/public/agentPresentation/AgentInstructionProfilePreview";
import { useEffect, useMemo, useRef, useState } from "react";
import { Archive, ArchiveRestore, ArrowUp, Loader2, Square, X } from "lucide-react";
import type {
  AgentConversationProjection,
  AgentConversationSummary,
  AgentUserInput,
} from "@research-assistant/core/agent";
import type { LlmModelSelection, PaperAiReadingRegion } from "@research-assistant/core/ai";
import type { LiteratureItem } from "@research-assistant/core/literature";
import { agentCatalogGateway, agentPaperContextGateway, agentPaperReadingHistoryMigrationGateway, agentPaperRecoveryGateway, agentPaperSubmissionGateway, agentSubtaskGateway } from "../../../../../../apps/web/src/core/public/agent";
import { agentSessionGateway } from "../../../../../../apps/web/src/core/public/agent";
import { getAiAssistanceGateway, aiAssistanceErrorMessage, type AiAssistanceGateway } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { getDoclingParserGateway, type DoclingParserGateway } from "../../../../../../apps/web/src/core/public/docling";
import { getLiteratureGateway } from "../../../../../../../plugins/ra-literature-library/src/services/literatureGateway";
import { notify } from "../../../../../../apps/web/src/core/public/feedback";
import type { ReaderCommand } from "@research-assistant/core/reader";
import type { ReaderCommandPayload } from "../../../../../../../plugins/ra-literature-library/src/features/reader/commands/ReaderCommandDispatcher";
import { agentConversationRequiresToolRecovery, agentTurnUnconfirmedChildCreations, applyAgentEvents, createAgentConversationProjection } from "@research-assistant/core/agent";
import { AgentChildCreationRecovery } from "../../../../../../apps/web/src/core/public/agentPresentation/AgentChildCreationRecovery";
import { childCreationFailureStage, isMatchingChildCreationFailure } from "../../../../../../apps/web/src/core/public/agentPresentation/childCreationFailure";
import { agentFailureDiagnostics } from "../../../../../../apps/web/src/core/public/agentPresentation/agentFailureDiagnostics";
import { startAgentHistoryFeed } from "@research-assistant/data/agent";
import { projectPaperAgentTurns } from "../../../../../../../plugins/ra-literature-library/src/features/reader/paperAgentProjection";
import { PaperAgentTurnEntryView } from "./PaperAgentTurnEntryView";
import { ReaderAgentModelControl } from "./ReaderAgentModelControl";
import { usePaperInputQueue } from "./usePaperInputQueue";
import { usePaperSteering } from "./usePaperSteering";
import { useAgentQueueAutoPrepare } from "../../../../../../apps/web/src/core/public/agentSubmission/useAgentQueueAutoPrepare";
import { PaperAiReadingSpeechInput } from "../../../../../../apps/web/src/core/public/speech";
import { paperAgentComposerAction } from "../../../../../../../plugins/ra-literature-library/src/features/reader/paperAgentComposer";
import { AgentResourcePickerMenu } from "../../../../../../apps/web/src/core/public/agentContext/AgentResourcePickerMenu";
import { useAgentComposerInput } from "../../../../../../apps/web/src/core/public/agentSubmission/useAgentComposerInput";
import { compactRunningMessage, dispatchQueuedAgentCommand, queuedCommandTransfer } from "../../../../../../apps/web/src/core/public/agentSubmission/dispatchQueuedAgentCommand";
import { readComposerCommand } from "../../../../../../apps/web/src/core/public/agentSubmission/agentComposerCommands";
import type { AgentCommandIntent } from "../../../../../../apps/web/src/core/public/agentSubmission/useAgentCommandPicker";
import { AgentSteeringReceipt } from "../../../../../../apps/web/src/core/public/agentSubmission/AgentSteeringReceipt";
import { reconcileSteeringTransfer } from "../../../../../../apps/web/src/core/public/agentSubmission/useAgentInputQueue";
import { useAgentDraftStore, useConversationDraft } from "../../../../../../apps/web/src/core/public/agentSubmission/AgentDraftProvider";

import { agentFailureHint } from "../../../../../../apps/web/src/core/public/agentPresentation/agentFailureHint";
import { clearAgentSessionSelection, readAgentSessionSelection, writeAgentSessionSelection } from "../../../../../../apps/web/src/core/public/agentConversations/agentSessionSelection";
import { PaperConceptRetrievalControl } from "../../../../../../apps/web/src/core/public/conceptRetrieval";
import { PaperConceptRetrievalTrace } from "./PaperConceptRetrievalTrace";
import { useAgentChildTasks } from "../../../../../../apps/web/src/core/public/agentPresentation/useAgentChildTasks";
import type { SpeechInputState } from "../../../../../../apps/web/src/core/public/speech";
import "../../../../../../../plugins/ra-literature-library/src/features/reader/PaperAiReadingPanel.css";
import { usePaperConversationCreation } from "./usePaperConversationCreation";
import { ConversationCreationRecovery } from "../../../../../../apps/web/src/core/public/agentConversations/ConversationCreationRecovery";
import { pendingBoundConversationId } from "../../../../../../apps/web/src/core/public/agentConversations/boundConversationCreationPending";
import { usePaperConversationArchives } from "./usePaperConversationArchives";

interface Props {
  readonly resourceGateways?: AgentResourceGateways;
  readonly item: LiteratureItem;
  readonly executeReaderCommand: (command: ReaderCommand, payload?: ReaderCommandPayload, target?: EventTarget | null) => boolean;
  readonly onHighlightContext?: (itemId: string, regions: readonly PaperAiReadingRegion[]) => void;
  readonly onClose: () => void;
  readonly hidden?: boolean;
  readonly aiGateway?: AiAssistanceGateway;
  readonly doclingGateway?: DoclingParserGateway;
}

import { acquirePaperConversationOwner, ensurePaperPreparation, setPaperOwnerPendingSubmission, setPaperOwnerPendingRetrieval, beginPaperOwnerWork, endPaperOwnerWork, type PaperConversationOwner, type PendingPaperSubmission, type PendingRetrievalSend } from "./paperConversationOwners";
import type {PaperPreparationSnapshot} from "./paperPreparation";
import {startPreparedPaper} from "./paperPreparedStart";
import {inspectPaperPreparation} from "./paperPreparationRecovery";
import {jumpPaperSource} from "./paperSourceNavigation";
import {interruptPaperTurn} from "./paperTurnActions";
export { resetPaperConversationOwnersForTests } from "./paperConversationOwners";


export function PaperAgentReadingPanel(props: Props): React.JSX.Element {
  return <ScopedPaperAgentReadingPanel key={props.item.id} {...props} />;
}

function ScopedPaperAgentReadingPanel({ item, resourceGateways, executeReaderCommand, onHighlightContext, onClose, hidden = false, aiGateway: suppliedAiGateway, doclingGateway: suppliedDoclingGateway }: Props): React.JSX.Element {
  const aiGateway = useMemo(() => suppliedAiGateway ?? getAiAssistanceGateway(), [suppliedAiGateway]);
  const doclingGateway = useMemo(() => suppliedDoclingGateway ?? getDoclingParserGateway(), [suppliedDoclingGateway]);
  const literatureGateway = useMemo(() => getLiteratureGateway(), []);
  const attachmentIds = useMemo(() => item.attachments.filter((attachment) => attachment.mimeType === "application/pdf").map((attachment) => attachment.id), [item.attachments]);
  const [conversations, setConversations] = useState<readonly AgentConversationSummary[]>([]);
  const [archivedView, setArchivedView] = useState(false);
  const paperBinding = useMemo(() => ({ kind: "paper" as const, resourceId: item.id }), [item.id]);
  const [conversationId, setConversationId] = useState<string | undefined>(() => readAgentSessionSelection({ kind: "paper", resourceId: item.id }) ?? undefined);
  const [projection, setProjection] = useState<AgentConversationProjection>();
  const [historySynchronized, setHistorySynchronized] = useState<string | undefined>();
  const { tasks: childTasks, reload: reloadChildTasks } = useAgentChildTasks(conversationId, projection?.activeTurn, (reason) => setError(aiAssistanceErrorMessage(reason)));
  const [context, setContext] = useState<Awaited<ReturnType<typeof agentPaperContextGateway.preview>>>();
  const [modelSelection, setModelSelection] = useState<LlmModelSelection>();
  const draftStore = useAgentDraftStore();
  const [message, setMessage, captureDraftAcknowledgment, draftResources] = useConversationDraft(draftStore, { hostKind: "paper", hostId: item.id, conversationId: conversationId ?? null });
  const composerInput=useRef<AgentComposerInputHandle>(null);
  const draftResourceId = useRef(crypto.randomUUID());
  const resourceScope={conversationId: conversationId ?? draftResourceId.current,kind:"paper" as const,resourceId:item.id};
  const inputResources=useAgentComposerInput({text:message,setText:setMessage,input:composerInput,access:draftResources,onCommand:submitCommand,onCommandFailure:()=>pauseRejectedSteerQueue(acquireOwner()),...(resourceScope?{scope:resourceScope}:{}),...(resourceGateways?{gateways:resourceGateways}:{})});
  const [queuedInputId, setQueuedInputId] = useState<string>();
  const [recoveryCandidates, setRecoveryCandidates] = useState<Awaited<ReturnType<typeof agentPaperRecoveryGateway.list>>["items"]>([]);
  const [pendingSubmission, setPendingSubmission] = useState<PendingPaperSubmission>();
  const [preparationState, setPreparationState] = useState<PaperPreparationSnapshot>();
  const [includeConceptRetrieval, setIncludeConceptRetrieval] = useState(false);
  const [pendingRetrieval, setPendingRetrieval] = useState<PendingRetrievalSend>();
  const [speechInputState, setSpeechInputState] = useState<SpeechInputState>("idle");
  const [workBusy, setBusy] = useState(false);
  const identityRef = useRef({ paperId: item.id, conversationId });
  const selectionRevision = useRef(0);
  const archiveOperations = usePaperConversationArchives({binding: paperBinding, conversationId, archivedView,
    selection: selectionRevision, identity: identityRef, setConversations, selectConversation});
  const archivePending = archiveOperations.pending;
  const busy = workBusy || inputResources.commandPending;
  const autoDispatchedSubmissionRef = useRef<string | null>(null);
  // This local value only bridges the short interval between the start receipt
  // and its first durable history batch. The projection is authoritative after
  // that, and the conversation identity keeps a previous selection from
  // steering or interrupting the newly selected conversation.
  const [localActiveTurn, setLocalActiveTurn] = useState<{ readonly conversationId: string; readonly turnId: string }>();
  const [retrievalTraceByTurn, setRetrievalTraceByTurn] = useState<Record<string, string>>({});
  const [error, setError] = useState<string>();
  const feedRef = useRef<ReturnType<typeof startAgentHistoryFeed> | undefined>(undefined);
  const feedGenerationRef = useRef(0);
  const sourceCacheRef = useRef(new Map<string, Awaited<ReturnType<typeof agentPaperSubmissionGateway.sources>>>());
  const paperOwnerRef = useRef<PaperConversationOwner | null>(null);
  const paperQueueScope = useMemo(() => conversationId === undefined ? null : ({ conversationId, paperId: item.id }), [conversationId, item.id]);
  const { controller: queueController, state: queueControllerState, retainTransfer, acquireOwner } = usePaperInputQueue(paperQueueScope, conversationId !== undefined);
  const queueState = queueControllerState?.queue ?? undefined;
  const projectionForConversation = conversationId !== undefined && projection?.conversationId === conversationId ? projection : undefined;
  const activeTurn = conversationId !== undefined && projectionForConversation !== undefined
    ? projectionForConversation.activeTurn !== null
      ? { conversationId, turnId: projectionForConversation.activeTurn }
      // Keep a just-started receipt visible until its TurnStarted event arrives;
      // once that turn is in the projection, the durable activeTurn is final.
      : localActiveTurn !== undefined && !projectionForConversation.turns.has(localActiveTurn.turnId) ? localActiveTurn : undefined
    : localActiveTurn?.conversationId === conversationId ? localActiveTurn : undefined;
  const toolRecoveryBlocked = activeTurn === undefined && !!projectionForConversation && agentConversationRequiresToolRecovery(projectionForConversation);
  const historyPending = conversationId !== undefined && historySynchronized !== conversationId;
  const preparationBlocked = !!preparationState && !["idle", "complete"].includes(preparationState.phase);
  const paperSteering = usePaperSteering(
    activeTurn === undefined || conversationId === undefined ? null : { conversationId: activeTurn.conversationId, paperId: item.id, turnId: activeTurn.turnId },
    () => undefined,
    historyPending ? null : projectionForConversation ?? null,
    conversationId === undefined ? undefined : { key: JSON.stringify(["paper", item.id, conversationId]), conversationId, captureAcknowledgment: captureDraftAcknowledgment, queueRejected: (request,ack)=>recoverRejectedSteer(acquireOwner(),request,ack??(()=>undefined)) },
    {state:queueControllerState,isReady:()=>{const queue=queueController.current?.getSnapshot();return !!queue?.queue&&!queue.loading&&!queue.editing&&!queue.uncertain&&!queue.error;}},
  );
  const autoPrepare = useAgentQueueAutoPrepare(
    activeTurn !== undefined,
    queueState?.inputs.find(input => !input.paused) ?? null,
    historyPending || toolRecoveryBlocked || preparationBlocked || archivePending || inputResources.commandPending || !!queueControllerState?.loading || !!queueControllerState?.editing || !!queueControllerState?.uncertain || !!paperSteering.state.awaitingHistory?.length || paperSteering.unresolved || busy || pendingSubmission !== undefined || activeTurn !== undefined
      || message.trim().length > 0 || queuedInputId !== undefined
      || queueState?.paused === true || queueState?.preparingInputId !== null,
    (input) => { const live=queueController.current?.getSnapshot();if(!live||live.loading||live.editing||live.uncertain||live.queue?.preparingInputId)return false; void send(undefined, input.text, input.inputId); },
  );
  function selectConversation(id: string | undefined): void {
    selectionRevision.current += 1;
    setConversationId(id);
  }
  const submissionGenerationRef = useRef(0);
  const activeRetrievalRequestId = useRef<string | undefined>(undefined);
  // Async work must never cross a paper switch or panel unmount.  This is a
  // host lifecycle token, separate from the conversation generation (which is
  // intentionally advanced when a new conversation is created).
  const paperEpochRef = useRef(0);
  const { creation, createConversation, captureAdoption } = usePaperConversationCreation({ paperId: item.id, conversationId,
    epoch: paperEpochRef, selectionRevision, onCreated: summary => {
      setConversations(current => [summary, ...current.filter(value => value.id !== summary.id)]);
      writeAgentSessionSelection(paperBinding, summary.id);
      draftStore.adoptPending({hostKind: "paper", hostId: item.id}, summary.id); selectConversation(summary.id);
    } });

  useEffect(() => {
    const lease = acquirePaperConversationOwner(item.id, conversationId);
    const owner = lease.owner;
    paperOwnerRef.current = owner;
    if (conversationId) ensurePaperPreparation(owner, item.id, conversationId, draftStore);
    const sync = () => {
      setPendingSubmission(owner.pendingSubmission);
      setPendingRetrieval(owner.pendingRetrieval);
      setPreparationState(owner.preparation?.getSnapshot());
    };
    owner.listeners.add(sync);
    sync();
    return () => {
      owner.listeners.delete(sync);
      lease.release();
      if (paperOwnerRef.current === owner) paperOwnerRef.current = null;
    };
  }, [item.id, conversationId, draftStore]);

  useEffect(() => {
    const epoch = paperEpochRef.current + 1;
    paperEpochRef.current = epoch;
    return () => {
      if (paperEpochRef.current === epoch) paperEpochRef.current += 1;
      activeRetrievalRequestId.current = undefined;
      feedRef.current?.stop();
    };
  }, [item.id]);

  useEffect(() => {
    submissionGenerationRef.current += 1;
    identityRef.current = { paperId: item.id, conversationId };
    // Busy is a view-local bridge for the selected conversation. Work owned by
    // another conversation remains in its owner registry and must not block the
    // newly selected conversation from recovering or dispatching its draft.
    setBusy(false);
    sourceCacheRef.current.clear();
    setContext(undefined);
    setRetrievalTraceByTurn({});
    activeRetrievalRequestId.current = undefined;
    setLocalActiveTurn(undefined);
  }, [conversationId, item.id]);

  useEffect(() => {
    let cancelled = false;
    setRecoveryCandidates([]);
    if (conversationId === undefined) return () => { cancelled = true; };
    void agentPaperRecoveryGateway.list({ conversationId, paperId: item.id, after: null, limit: 20 })
      .then((page) => { if (!cancelled) setRecoveryCandidates(page.items); })
      .catch((reason: unknown) => { if (!cancelled) setError(`恢复未发送请求：${aiAssistanceErrorMessage(reason)}`); });
    return () => { cancelled = true; };
  }, [conversationId, item.id]);

  useEffect(() => {
    let cancelled = false;
    const initialSelectionRevision = selectionRevision.current;
    void agentPaperReadingHistoryMigrationGateway.migrate({ paperId: item.id })
      .catch((reason: unknown) => {
        if (cancelled) return;
        const message = aiAssistanceErrorMessage(reason);
        // Legacy history is an import sidecar. A damaged or unsupported legacy
        // row must remain visible as a warning, but it must not block a new
        // shared Agent conversation for the current paper.
        notify.warning(`旧阅读历史迁移未完成：${message}。当前共享 Agent 仍可使用。`);
      })
      .then(() => agentCatalogGateway.list({ archived: archivedView, limit: 50, before: null, binding: { kind: "paper", resourceId: item.id } }))
      .then((page) => {
        if (cancelled) return;
        if (selectionRevision.current !== initialSelectionRevision) {
          // A late catalog may add history, but cannot undo an explicit selection
          // or remove a conversation created while that catalog was loading.
          setConversations(current => [...current, ...page.conversations.filter(entry => !current.some(saved => saved.id === entry.id))]);
          return;
        }
        setConversations(page.conversations);
        const remembered = readAgentSessionSelection(paperBinding);
        const selected = page.conversations.find((conversation) => conversation.id === remembered)?.id
          ?? (!remembered && pendingBoundConversationId(paperBinding) ? undefined : page.conversations[0]?.id);
        setConversationId(selected);
        if (selected) writeAgentSessionSelection(paperBinding, selected);
        else clearAgentSessionSelection(paperBinding);
      })
      .catch((reason: unknown) => { if (!cancelled) setError(aiAssistanceErrorMessage(reason)); });
    return () => { cancelled = true; };
  }, [item.id, paperBinding, archivedView]);

  useEffect(() => {
    const generation = feedGenerationRef.current + 1;
    feedGenerationRef.current = generation;
    const id = conversationId;
    setHistorySynchronized(undefined);
    feedRef.current?.stop();
    feedRef.current = undefined;
    if (id === undefined) { setProjection(undefined); return; }
    const initial = createAgentConversationProjection(id);
    setProjection(initial);
    const feed = startAgentHistoryFeed(agentSessionGateway, {
      conversationId: id,
      afterSeq: 0,
      onSyncState: ready => { if (feedGenerationRef.current === generation) setHistorySynchronized(ready ? id : undefined); },
      // A stopped feed may still have one in-flight batch.  Do not apply that
      // batch to the projection for a newly selected conversation.
      onBatch: (events) => setProjection((current) => {
        if (feedGenerationRef.current !== generation) return current ?? initial;
        if (current !== undefined && current.conversationId !== id) return current;
        return applyAgentEvents(current ?? initial, events);
      }),
    });
    feedRef.current = feed;
    void feed.done.catch((reason: unknown) => {
      if (feedRef.current !== feed) return;
      feedRef.current = undefined;
      setHistorySynchronized(undefined);
      setError(`读取会话历史：${aiAssistanceErrorMessage(reason)}`);
    });
    return () => {
      if (feedGenerationRef.current === generation) feedGenerationRef.current += 1;
      feed.stop();
      if (feedRef.current === feed) feedRef.current = undefined;
    };
  }, [conversationId]);

  // The reader panel is hidden with the native `hidden` attribute and remains
  // mounted. If its history feed failed while hidden, reopening must reconnect
  // the same conversation instead of requiring a different selection.
  useEffect(() => {
    if (hidden || conversationId === undefined || feedRef.current !== undefined) return;
    const generation = feedGenerationRef.current + 1;
    feedGenerationRef.current = generation;
    const id = conversationId;
    const initial = projection?.conversationId === id ? projection : createAgentConversationProjection(id);
    const feed = startAgentHistoryFeed(agentSessionGateway, {
      conversationId: id,
      afterSeq: initial.seq,
      onSyncState: ready => { if (feedGenerationRef.current === generation) setHistorySynchronized(ready ? id : undefined); },
      onBatch: (events) => setProjection((current) => {
        if (feedGenerationRef.current !== generation) return current ?? initial;
        if (current !== undefined && current.conversationId !== id) return current;
        return applyAgentEvents(current ?? initial, events);
      }),
    });
    feedRef.current = feed;
    void feed.done.catch((reason: unknown) => {
      if (feedRef.current !== feed) return;
      feedRef.current = undefined;
      setHistorySynchronized(undefined);
      setError(`读取会话历史：${aiAssistanceErrorMessage(reason)}`);
    });
    return () => {
      if (feedGenerationRef.current === generation) feedGenerationRef.current += 1;
      feed.stop();
      if (feedRef.current === feed) feedRef.current = undefined;
    };
  }, [hidden, conversationId]);


  useEffect(() => {
    const active = activeTurn;
    if (active === undefined) return;
    const turn = projection?.turns.get(active.turnId);
    if (turn?.outcome !== null && turn !== undefined) setLocalActiveTurn(undefined);
  }, [activeTurn, projection]);

  const sourceTurnIds = JSON.stringify(projection?.turnOrder ?? []);
  useEffect(() => {
    if (conversationId === undefined) return;
    let cancelled = false;
    const turnIds = JSON.parse(sourceTurnIds) as readonly string[];
    void Promise.all(turnIds.map(async (turnId) => {
      const cacheKey = `${conversationId}:${turnId}`;
      const result = sourceCacheRef.current.get(cacheKey)
        ?? await agentPaperSubmissionGateway.sources({ conversationId, paperId: item.id, turnId });
      sourceCacheRef.current.set(cacheKey, result);
      return [turnId, result.retrievalTraceId] as const;
    })).then((entries) => {
      if (cancelled) return;
      setRetrievalTraceByTurn((current) => {
        const next = { ...current };
        for (const [turnId, traceId] of entries) {
          if (traceId !== undefined) next[turnId] = traceId;
        }
        return next;
      });
    }).catch(() => {
      // Source geometry is optional for rendering history; jump-to-source reports its own error.
    });
    return () => { cancelled = true; };
  }, [conversationId, item.id, sourceTurnIds]);

  async function send(retrievalTraceId?: string, explicitText?: string, queuedInputIdOverride?: string, frozenInput?: readonly AgentUserInput[],
    frozenSubmission?: {readonly conversationId:string; readonly requestId:string}): Promise<void> {
    if (archivePending || historyPending || toolRecoveryBlocked || preparationBlocked) return;
    return questionSubmission.send(retrievalTraceId, explicitText, queuedInputIdOverride, frozenInput, frozenSubmission);
  }

  function submitCommand(text: string, intent: AgentCommandIntent): void {
    if (archivePending || historyPending || toolRecoveryBlocked || preparationBlocked) return;
    if (workBusy || selectedConversation?.archived || paperSteering.unresolved || pendingSubmission || pendingRetrieval) return;
    if ((activeTurn || inputResources.commandPending) && intent === "steer") { notify.error(compactRunningMessage); return; }
    if (activeTurn || inputResources.commandPending || draftResources.draft().queuePlacement) void enqueue(text, message, []);
    else void inputResources.executeCommand(message);
  }
  async function enqueue(text: string, originalDraft = text, commandInput?: readonly AgentUserInput[]): Promise<void> {
    if (archivePending || preparationBlocked) return;
    return questionSubmission.enqueue(text, originalDraft, commandInput);
  }

  function confirmConceptRetrieval(): void {
    const pending = pendingRetrieval;
    if (pending?.preview === undefined || busy) return;
    const owner=paperOwnerRef.current;
    if (owner) setPaperOwnerPendingRetrieval(owner,undefined); else setPendingRetrieval(undefined);
    setMessage(pending.text);
    void send(pending.preview.traceId, pending.text,undefined,pending.userInput,pending.submissionOwner);
  }

  function cancelConceptRetrieval(): void {
    const pending = pendingRetrieval;
    if (pending === undefined) return;
    activeRetrievalRequestId.current = undefined;
    const owner=paperOwnerRef.current;
    if (owner) setPaperOwnerPendingRetrieval(owner,undefined); else setPendingRetrieval(undefined);
    draftResources.restore(pending.text,pending.userInput);
    if (pending.isLoading) {
      void aiGateway.cancelConceptRetrievalPreview(pending.requestId).catch(() => undefined);
    }
  }

  async function startPreparedSubmission(pending: PendingPaperSubmission, owner: PaperConversationOwner | null, queueAtConfirm=queueController.current, queueStateAtConfirm=queueState): Promise<void> {
    const owns = () => identityRef.current.paperId === item.id && identityRef.current.conversationId === pending.preview.conversationId;
    await startPreparedPaper(pending, owner?.preparation ?? null, queueAtConfirm, queueStateAtConfirm, started => {
      if (!owns()) return;
      if (pending.retrievalTraceId !== undefined) setRetrievalTraceByTurn(current => ({...current, [started.turnId]: pending.retrievalTraceId!}));
      if (started.outcome === null) setLocalActiveTurn({conversationId: pending.preview.conversationId, turnId: started.turnId});
      const next: LlmModelSelection = {providerId: pending.preview.input.model.providerId as LlmModelSelection["providerId"], modelId: pending.preview.input.model.modelId};
      if (pending.preview.input.reasoningEffort !== null) Object.assign(next, {reasoningEffort: pending.preview.input.reasoningEffort});
      if (pending.preview.input.serviceTier) Object.assign(next, {serviceTier: pending.preview.input.serviceTier});
      setModelSelection(next);
    }, () => {if (owns()) setQueuedInputId(undefined);});
    notify.success("已提交到共享阅读 Agent");
  }

  async function confirmPendingSubmission(pendingOverride?: PendingPaperSubmission): Promise<void> {
    const pending = pendingOverride ?? pendingSubmission;
    if (historyPending || toolRecoveryBlocked || pending === undefined || (pendingOverride === undefined && busy)) return;
    const owner=paperOwnerRef.current;
    const queueAtConfirm=queueController.current,queueStateAtConfirm=queueState;
    setBusy(true); setError(undefined);
    if (owner) beginPaperOwnerWork(owner);
    if (owner) setPaperOwnerPendingSubmission(owner,undefined); else setPendingSubmission(undefined);
    try {
      await startPreparedSubmission(pending,owner,queueAtConfirm,queueStateAtConfirm);
    } catch (reason: unknown) {
      const stillOwns = identityRef.current.paperId === pending.preview.paperId && identityRef.current.conversationId === pending.preview.conversationId;
      if (owner) setPaperOwnerPendingSubmission(owner,pending); else if (stillOwns) setPendingSubmission(pending);
      if (stillOwns) {
        if (!owner?.preparation?.unresolved()) setMessage(pending.text);
        const message = aiAssistanceErrorMessage(reason); setError(message); notify.error(message);
      }
    } finally { if (owner) endPaperOwnerWork(owner.key, owner); if (identityRef.current.paperId === pending.preview.paperId && identityRef.current.conversationId === pending.preview.conversationId) setBusy(false); }
  }

  useEffect(() => {
    const pending = pendingSubmission;
    if (historyPending || toolRecoveryBlocked || pending === undefined || pending.manualConfirmation || conversations.find((conversation) => conversation.id === conversationId)?.archived
      || pending.preview.conversationId !== conversationId
      || autoDispatchedSubmissionRef.current === pending.preview.requestId) return;
    autoDispatchedSubmissionRef.current = pending.preview.requestId;
    void confirmPendingSubmission(pending);
  }, [historyPending, toolRecoveryBlocked, conversationId, conversations, pendingSubmission]);

  async function restoreRecovery(candidate: (typeof recoveryCandidates)[number]): Promise<void> {
    if (busy || pendingSubmission !== undefined || conversationId === undefined) return;
    const owner=paperOwnerRef.current;
    if (owner) beginPaperOwnerWork(owner);
    setBusy(true); setError(undefined);
    try {
      const preview = await agentPaperSubmissionGateway.restore({ conversationId, requestId: candidate.requestId, paperId: item.id, paperContextSnapshotId: candidate.paperContextSnapshotId });
      const retrievalSource = preview.input.context.find((source) => source.kind === "retrieval");
      const restored={ preview, text: preview.input.text, sourceCount: preview.input.context.length, imageCount: 0, ...(retrievalSource === undefined ? {} : { retrievalTraceId: retrievalSource.sourceId.slice("retrieval:".length) }) } satisfies PendingPaperSubmission;
      if (owner) setPaperOwnerPendingSubmission(owner,restored); else setPendingSubmission(restored);
      setRecoveryCandidates((current) => current.filter((entry) => entry.requestId !== candidate.requestId));
      notify.info("已恢复未启动的论文提交，请确认后发送");
    } catch (reason: unknown) {
      const message = aiAssistanceErrorMessage(reason); setError(message); notify.error(message);
    } finally { if (owner) endPaperOwnerWork(owner.key,owner);setBusy(false); }
  }

  function cancelPendingSubmission(): void {
    if (pendingSubmission === undefined || busy) return;
    if (paperOwnerRef.current?.preparation?.unresolved()) return;
    setMessage(pendingSubmission.text);
    const owner=paperOwnerRef.current;
    if (owner) setPaperOwnerPendingSubmission(owner,undefined); else setPendingSubmission(undefined);
    notify.info("已取消本次论文上下文发送");
  }

  async function checkPaperPreparation(): Promise<void> {
    const owner = paperOwnerRef.current, preparation = owner?.preparation;
    if (!owner || !preparation || busy || historyPending || toolRecoveryBlocked) return;
    beginPaperOwnerWork(owner); setBusy(true); setError(undefined);
    try {
      const result = await inspectPaperPreparation(owner, queueController.current, (conversation, turn) => {
        if (identityRef.current.conversationId === conversation) setLocalActiveTurn({conversationId: conversation, turnId: turn});
      });
      if (result === "complete" && identityRef.current.conversationId === conversationId) notify.info("已核对原论文提交");
    } catch (reason: unknown) {if (identityRef.current.conversationId === conversationId) {const message = aiAssistanceErrorMessage(reason); setError(message); notify.error(message);}}
    finally {endPaperOwnerWork(owner.key, owner); if (identityRef.current.conversationId === conversationId) setBusy(false);}
  }

  async function steerQueuedInput(inputId: string, text: string): Promise<boolean> {
    if (historyPending || inputResources.commandPending || !activeTurn || workBusy || paperSteering.state.awaitingHistory?.length || paperSteering.unresolved) return false;
    if (readComposerCommand(text)) {
      return await dispatchQueuedAgentCommand({inputId, text, running: true, execution: {compact: inputResources.executeCommand},
        transfer: () => queuedCommandTransfer(queueController, inputId, text, () => retainTransfer({type: "remove", inputId}))}) ?? false;
    }
    const selectionAtStart = selectionRevision.current;
    const transfer = retainTransfer({ type: "remove", inputId });
    if (!transfer) return false;
    setBusy(true); setError(undefined);
    let retained=false;
    try {
      const accepted = await paperSteering.submit(text, {
        confirmed: () => transfer.apply(),
        rejected: transfer.release,
        queueRejected: async () => {retained=await prioritizeRejectedSteerLease(acquireOwner(),inputId);return retained;},
        userInput: queueState?.inputs.find(input=>input.inputId===inputId)?.userInput ?? [],
      });
      if (selectionRevision.current !== selectionAtStart) return false;
      if(!accepted&&retained)return true;
      if (!accepted) throw new Error("当前回答暂不可插入，消息仍保留在队列。");
      notify.success("已将队列消息补充到当前回答");
      return true;
    } catch (reason: unknown) {
      if (selectionRevision.current !== selectionAtStart) return false;
      const message = aiAssistanceErrorMessage(reason);
      setError(message); notify.error(message);
      return false;
    } finally { if (selectionRevision.current === selectionAtStart) setBusy(false); }
  }

  async function interrupt(): Promise<void> {
    await interruptPaperTurn(activeTurn, selectionRevision, setError);
  }

  async function jumpToContext(turnId: string, sourceId: string): Promise<void> {
    await jumpPaperSource({paperId: item.id, conversationId, turnId, sourceId, context, cache: sourceCacheRef.current,
      identity: identityRef, selection: selectionRevision, execute: executeReaderCommand, highlight: onHighlightContext});
  }

  async function resumeChildTask(childConversationId: string): Promise<void> {
    if (!conversationId || busy) return;
    setBusy(true);
    try {
      await agentSubtaskGateway.resumePaper(conversationId, childConversationId, item.id);
      notify.success("已恢复论文子任务，正在继续执行。");
      await reloadChildTasks();
    } catch (reason) {
      setError(aiAssistanceErrorMessage(reason));
      notify.error("无法恢复论文子任务：会话绑定或冻结提交已不可用。");
    } finally { setBusy(false); }
  }

  async function changeConversationArchive(conversation: AgentConversationSummary): Promise<void> {
    await archiveOperations.change(conversation);
  }

  const selectedConversation = conversationId === undefined ? undefined : conversations.find((conversation) => conversation.id === conversationId);

  const questionSubmission = createPaperQuestionSubmission({ item, conversationId, message, inputResources, draftResources,
    busy: busy || archivePending, workBusy, selectedConversation, paperSteering, speechInputState, pendingSubmission, pendingRetrieval,
    activeTurn, includeConceptRetrieval, aiGateway, doclingGateway, literatureGateway, attachmentIds, modelSelection,
    paperEpochRef, paperOwnerRef, selectionRevision, activeRetrievalRequestId, submissionGenerationRef, identityRef,
    queueController, queueControllerState, queueState, retainTransfer, queuedInputId, autoPrepare, captureDraftAcknowledgment, draftStore,
    createConversation, setBusy, setError, setMessage, setPendingRetrieval, setContext });
  function newConversation(): void {
    if (creation.state?.adopted || (conversationId && creation.state?.intent?.id === conversationId)) {
      notify.error("当前创建恢复记录尚未清理，请先处理后再新建对话。"); return;
    }
    clearAgentSessionSelection(paperBinding, conversationId); selectConversation(undefined);
  }
  const turns = projection === undefined ? [] : projectPaperAgentTurns(projection);
  return <aside aria-label="AI 文献阅读" className="paper-ai-reading-panel" hidden={hidden}>
    <header><div><strong>AI 文献阅读 · 共享 Agent</strong><span>{conversations.length} 个对话</span></div><ControlHint content="关闭 AI 阅读"><button aria-label="关闭 AI 阅读" onClick={onClose}  type="button"><X /></button></ControlHint></header>
    <div className="paper-ai-reading-session-bar"><div className="paper-ai-reading-session-filters" role="group" aria-label="对话状态"><button aria-pressed={!archivedView} disabled={busy} onClick={() => setArchivedView(false)} type="button">当前</button><button aria-pressed={archivedView} disabled={busy} onClick={() => setArchivedView(true)} type="button">已归档</button></div><label>会话<select disabled={busy} onChange={(event) => { const next = event.target.value || undefined; if (next) {writeAgentSessionSelection(paperBinding, next);selectConversation(next);} else newConversation(); }} value={conversationId ?? ""}><option value="">{archivedView ? "选择归档会话" : "新会话"}</option>{conversations.map((conversation) => <option key={conversation.id} value={conversation.id}>{conversation.title}</option>)}{childTasks.filter((task) => !conversations.some((conversation) => conversation.id === task.childConversationId)).map((task) => <option key={task.childConversationId} value={task.childConversationId}>子任务 · {task.status}</option>)}</select></label><div className="paper-ai-reading-session-actions">{selectedConversation !== undefined && <ControlHint content={archivedView ? "恢复当前对话" : "归档当前对话"}><button aria-label={archivedView ? "恢复当前对话" : "归档当前对话"} disabled={busy || archivePending} onClick={() => void changeConversationArchive(selectedConversation)}  type="button">{archivedView ? <ArchiveRestore aria-hidden="true" /> : <Archive aria-hidden="true" />}</button></ControlHint>}{!archivedView && <button disabled={busy} onClick={newConversation} type="button">新会话</button>}</div></div>
    <ConversationCreationRecovery {...creation} captureAdoption={captureAdoption}/>
    {childTasks.length > 0 && <section aria-label="子任务" className="paper-ai-reading-child-tasks"><strong>子任务</strong><ul>{childTasks.map((task) => <li key={task.childConversationId}><button disabled={busy} onClick={() => { writeAgentSessionSelection(paperBinding, task.childConversationId); selectConversation(task.childConversationId); }} type="button"><span>深度 {task.depth} · {task.status}</span><small>{task.modelCalls} 次模型 / {task.toolCalls} 次工具预算</small></button>{task.status === "recovery_required" && <ControlHint content="使用冻结提交恢复论文子任务"><button disabled={busy} onClick={() => void resumeChildTask(task.childConversationId)} type="button" >恢复并继续</button></ControlHint>}</li>)}</ul></section>}
    <AgentSteeringReceipt awaitingHistory={paperSteering.state.awaitingHistory} transferFailures={paperSteering.state.transferFailures} onRetryTransfer={id=>void reconcileSteeringTransfer(queueController.current,id,paperSteering.retryConfirmation)} uncertain={paperSteering.uncertain} pending={paperSteering.pending} text={paperSteering.state.request?.text} status={paperSteering.status} {...(paperSteering.state.request?.inputId ? { requestId: paperSteering.state.request.inputId } : {})} disabled={busy || historyPending || archivePending || selectedConversation?.archived===true || projection===undefined} onRetry={() => void paperSteering.retry()} />
    {paperSteering.state.phase === "rejected" && <p role="alert" className="paper-ai-reading-error">{paperSteering.state.status}</p>}
    {selectedConversation?.archived && <p className="paper-ai-reading-archived" role="status">此对话已归档，历史可阅读；恢复后才能继续发送。</p>}
    {error !== undefined && <p className="paper-ai-reading-error">{error}</p>}
    {preparationBlocked && !pendingSubmission && <section className="paper-ai-reading-recovery" aria-label="论文提交待核对">
      <strong>论文提交待核对</strong><p>原请求已保留；结果未确认前不会重新发送。{preparationState?.intent ? ` 请求：${preparationState.intent.requestId}` : " 本地记录无法读取。"}{preparationState?.requireProof && ["unknown", "unconfirmed", "localError"].includes(preparationState.phase) && (!preparationState.prepareProof && !preparationState.prepareEpoch || preparationState.cause instanceof Error && preparationState.cause.message === "PAPER_PREPARE_ORIGINAL_PROOF_UNCONFIRMED") ? " 原数据环境证明无法确认，暂不能继续或重发。" : ""}</p>
      <button type="button" disabled={busy || historyPending || toolRecoveryBlocked || !preparationState?.intent || selectedConversation?.archived === true} onClick={() => void checkPaperPreparation()}>核对原请求</button>
    </section>}
    {pendingSubmission !== undefined && <section className="paper-ai-reading-context-confirmation" aria-label="论文上下文已冻结">
      <strong>论文上下文已冻结</strong>
      <p>将使用已冻结的论文解析快照（{pendingSubmission.sourceCount} 个来源{pendingSubmission.imageCount > 0 ? `，${pendingSubmission.imageCount} 个包含图片` : ""}）。</p>
      <details><summary>查看本次消息和完整来源范围</summary><p>本次消息</p><pre>{pendingSubmission.text}</pre>
        <AgentInstructionProfilePreview input={pendingSubmission.preview.input} subject="论文" />
        <p>当前论文文件不会自动外发其他内容。</p>
      </details>
      {error !== undefined || pendingSubmission.manualConfirmation ? <div>{error !== undefined && <p role="alert">{error}</p>}<button type="button" disabled={busy || historyPending || toolRecoveryBlocked} onClick={() => { autoDispatchedSubmissionRef.current = null; void confirmPendingSubmission(); }}>{pendingSubmission.manualConfirmation && !preparationState?.start ? "继续发送" : preparationBlocked ? "核对原启动" : "重试发送"}</button>{!preparationBlocked && <button type="button" disabled={busy} onClick={cancelPendingSubmission}>取消</button>}</div> : <p role="status">{historyPending ? "正在核对对话历史，已保留原提交。" : toolRecoveryBlocked ? "工具操作尚待核对，已保留原提交。" : "正在发送到共享 Agent…"}</p>}
    </section>}
    {recoveryCandidates.length > 0 && <section className="paper-ai-reading-recovery" aria-label="恢复未启动的提交">
      <strong>有 {recoveryCandidates.length} 条未启动的提交</strong>
      <p>应用上次关闭前已冻结论文上下文，但尚未启动模型。请选择要恢复的消息。</p>
      <ul>{recoveryCandidates.map((candidate) => <li key={candidate.requestId}><span>{candidate.messageExcerpt}</span><button type="button" disabled={busy || preparationBlocked || pendingSubmission !== undefined} onClick={() => void restoreRecovery(candidate)}>恢复预览</button></li>)}</ul>
    </section>}
    <div className="paper-ai-reading-messages">
      {queueControllerState && <AgentInputQueue state={queueControllerState} disabled={historyPending || !!paperSteering.state.awaitingHistory?.length || workBusy || preparationBlocked || paperSteering.unresolved || pendingSubmission !== undefined || pendingRetrieval !== undefined || selectedConversation?.archived === true}
        compaction={inputResources.commandActivities.find(item => item.state === "pending")}
        onEdit={edit => queueController.current?.edit(edit) ?? Promise.resolve(false)}
        onEditInComposer={(input, index, current) => {draftResources.restoreQueue(input, index, current); return queueController.current?.edit({type: "remove", inputId: input.inputId}) ?? Promise.resolve(false);}}
        onReload={() => queueController.current?.reload()} onRetry={() => void queueController.current?.retry()}
        {...(activeTurn || inputResources.commandPending ? {onSteer: steerQueuedInput} : {})} />}
      {turns.length === 0 && <p className="empty">首次发送会冻结当前论文解析快照，并写入共享 Agent 历史。</p>}{turns.map((turn) => {
        const projectedTurn = projectionForConversation?.turns.get(turn.id);
        const children = projectionForConversation?.activeTurn === null && projectedTurn ? agentTurnUnconfirmedChildCreations(projectedTurn) : [];
        const matchingChild = conversationId ? children.find(child => isMatchingChildCreationFailure(turn.failureReason, child, conversationId, turn.id)) : undefined;
        return <article key={turn.id}>{turn.entries.map((entry) => <PaperAgentTurnEntryView key={entry.kind === "message" ? entry.message.id : entry.group.id} entry={entry} readMessageData={message => conversationId ? readMessageInspection({ conversationId, turnId: turn.id, itemId: message.id }) : Promise.resolve(null)} onJump={(sourceId) => void jumpToContext(turn.id, sourceId)} />)}{turn.reconnecting && <p role="status">正在重新连接…</p>}{turn.unconfirmedReceipts && <div className="paper-ai-reading-error" role="alert">工具结果的原操作回执尚未确认，不能继续此对话或回滚；请先核对原操作，不要重复执行。轮次：{turn.id}；待确认调用：{turn.unconfirmedReceipts.join("、")}</div>}
          {conversationId && projectionForConversation && children.map(child => <AgentChildCreationRecovery key={child.callId} identity={{...child, conversationId, turnId: turn.id, watermark: projectionForConversation.seq}} {...(child === matchingChild && projectedTurn ? {diagnosticText: agentFailureDiagnostics(conversationId, turn.id, projectedTurn), failureStage: childCreationFailureStage(turn.failureReason, child, conversationId, turn.id)} : {})} />)}
          {turn.failureReason !== undefined && !matchingChild && <div className="paper-ai-reading-error" role="alert"><p>Agent 请求失败{turn.failureCategory === "providerTemporaryUnavailable" ? "（提供方暂时不可用，可重试）" : turn.failureCategory === "toolRound" ? "（工具回合失败）" : turn.failureCategory === "persistence" ? "（保存失败，需要恢复）" : turn.failureCategory === "admission" ? "（请求未被接纳）" : turn.failureCategory === "recovery" ? "（需要恢复）" : ""}：{turn.failureReason}</p>{agentFailureHint(turn.failureReason)&&<p>{agentFailureHint(turn.failureReason)}</p>}</div>}{retrievalTraceByTurn[turn.id] !== undefined && <PaperConceptRetrievalTrace aiGateway={aiGateway} traceId={retrievalTraceByTurn[turn.id]!} />}<span className="paper-ai-reading-turn-model">{turn.providerId} · {turn.modelId}</span></article>;
      })}
      <AgentCompactionActivity records={inputResources.commandActivities} />
    </div>
    {conversationId && <SystemDocumentApprovalPanel conversationId={conversationId}/>}
    <footer className="paper-ai-reading-composer agent-reference-anchor">{historyPending ? <p role="status">正在核对对话历史，请稍候。</p> : toolRecoveryBlocked && <p role="status">工具操作尚待核对，请勿重复提交。</p>}{inputResources.menu}{inputResources.warning && <p role="alert">{inputResources.warning}</p>}<PaperConceptRetrievalControl checked={includeConceptRetrieval} disabled={busy || archivePending || activeTurn !== undefined || pendingRetrieval !== undefined || selectedConversation?.archived === true} onCancel={cancelConceptRetrieval} onChange={setIncludeConceptRetrieval} onConfirm={confirmConceptRetrieval} {...(pendingRetrieval === undefined ? {} : { pending: pendingRetrieval })} /><AgentComposerTextInput key={draftResources.draft().queuePlacement?.input.inputId ?? "new"} inputRef={composerInput} aria-label="向共享 AI 提问当前文献" disabled={workBusy || archivePending || selectedConversation?.archived === true} onChange={inputResources.change} onKeyDown={(event) => { if(inputResources.keyDown(event))return; if(inputResources.commandPending&&event.key==="Enter"&&(event.ctrlKey||event.metaKey)){event.preventDefault();return;} const action = paperAgentComposerAction({ key: event.key, shiftKey: event.shiftKey, isComposing: event.nativeEvent.isComposing || event.keyCode === 229, activeTurn: activeTurn !== undefined || inputResources.commandPending, hasText: message.trim().length > 0, speechIdle: speechInputState === "idle" }); if (action === "none") return; event.preventDefault(); if (action === "interrupt") void interrupt(); else void send(); }} placeholder="询问当前文献；Enter 发送，Shift+Enter 换行；活动回答中 Enter 补充" rows={4} draft={inputResources.draft} onDraftChange={inputResources.changeDraft} onCaretChange={inputResources.update}  {...(resourceGateways ? {gateways: resourceGateways} : {})} {...(resourceScope ? {scope: resourceScope} : {})} /><div className="paper-ai-reading-composer-actions"><ReaderAgentModelControl disabled={busy || archivePending || activeTurn !== undefined || selectedConversation?.archived === true} gateway={aiGateway} onSelection={setModelSelection} selection={modelSelection} /><AgentResourcePickerMenu {...(resourceGateways?{gateways:resourceGateways}:{})} {...(resourceScope?{scope:resourceScope}:{})} onSelect={inputResources.select} disabled={busy || archivePending || selectedConversation?.archived === true} /><PaperAiReadingSpeechInput disabled={busy || archivePending || hidden || selectedConversation?.archived === true} onChange={inputResources.change} onStateChange={setSpeechInputState} value={inputResources.value} /><button aria-label={activeTurn === undefined || message.trim() ? "发送" : "中断当前回答"} className="paper-ai-reading-send" disabled={historyPending || toolRecoveryBlocked || (preparationBlocked && (activeTurn === undefined || !!message.trim())) || archivePending || selectedConversation?.archived === true || (workBusy || pendingRetrieval !== undefined ? true : activeTurn === undefined ? message.trim().length === 0 || speechInputState !== "idle" : false)} onClick={() => activeTurn === undefined || message.trim() ? void send() : void interrupt()}  type="button">{workBusy ? <Loader2 className="spin" aria-hidden="true" /> : activeTurn !== undefined && !message.trim() ? <Square aria-hidden="true" /> : <ArrowUp aria-hidden="true" />}</button></div></footer>
  </aside>;
}

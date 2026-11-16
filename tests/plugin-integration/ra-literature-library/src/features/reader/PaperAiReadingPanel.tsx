import { ControlHint } from "../../../../../../apps/web/src/core/public/workbench";
import { useEffect, useMemo, useRef, useState } from "react";
import { isDesktopRuntime } from "../../../../../../apps/web/src/core/public/runtimeEnvironment";
import { listen } from "../../../../../../apps/web/src/core/public/platform";
import {
  ArrowUp,
  ChevronDown,
  CornerUpRight,
  Loader2,
  Pencil,
  Square,
  Trash2,
  X,
} from "lucide-react";
import type {
  LlmModelSelection,
  PaperAiReadingPrepareResult,
  PaperAiReadingMessage,
  PaperAiReadingRegion,
  PaperAiReadingSessionDetail,
  PaperAiReadingSessionSummary,
  PaperAiReadingStreamEvent,
  PaperReadingBootstrapOptions,
} from "@research-assistant/core/ai";
import type { LiteratureItem } from "@research-assistant/core/literature";

import {
  getAiAssistanceGateway,
  aiAssistanceErrorMessage,
  type AiAssistanceGateway,
} from "../../../../../../apps/web/src/core/public/aiAssistance";
import {
  getDoclingParserGateway,
  type DoclingParserGateway,
} from "../../../../../../apps/web/src/core/public/docling";
import {
  getLiteratureGateway,
  type LiteratureGateway,
} from "../../../../../../../plugins/ra-literature-library/src/services/literatureGateway";
import { notify } from "../../../../../../apps/web/src/core/public/feedback";
import type { ReaderCommand } from "@research-assistant/core/reader";
import type { ReaderCommandPayload } from "../../../../../../../plugins/ra-literature-library/src/features/reader/commands/ReaderCommandDispatcher";
import { renderPdfRegionPng } from "../../../../../../../plugins/ra-literature-library/src/features/reader/pdf/pdfRegionScreenshot";
import { PaperAiReadingMarkdown } from "../../../../../../../plugins/ra-literature-library/src/features/reader/PaperAiReadingMarkdown";
import { PaperAiReadingDebugData } from "./PaperAiReadingDebugData";
import {
  latestUsageTurn,
  PaperAiReadingContextIndicator,
  PaperAiReadingModelControl,
} from "./PaperAiReadingComposerControls";
import {
  PaperAiReadingSpeechInput,
  speechInputStateLabel,
} from "../../../../../../apps/web/src/core/public/speech";
import type { SpeechInputState } from "../../../../../../apps/web/src/core/public/speech";
import "../../../../../../../plugins/ra-literature-library/src/features/reader/PaperAiReadingPanel.css";
import {
  PaperReadingBootstrapBadge,
  PaperReadingBootstrapControl,
} from "../../../../../../../plugins/ra-literature-library/src/features/reader/PaperReadingBootstrapControl";
import {
  PaperConceptRetrievalControl,
  type PendingConceptRetrieval,
} from "../../../../../../apps/web/src/core/public/conceptRetrieval";
import { PaperConceptRetrievalTrace } from "./PaperConceptRetrievalTrace";

interface PaperAiReadingPanelProps {
  readonly aiGateway?: AiAssistanceGateway;
  readonly doclingGateway?: DoclingParserGateway;
  readonly item: LiteratureItem;
  readonly executeReaderCommand: (command: ReaderCommand, payload?: ReaderCommandPayload, target?: EventTarget | null) => boolean;
  readonly literatureGateway?: LiteratureGateway;
  readonly onHighlightContext?: (itemId: string, regions: readonly PaperAiReadingRegion[]) => void;
  readonly onClose: () => void;
  readonly hidden?: boolean;
}

interface StreamingTurnState {
  readonly clientRequestId: string;
  readonly messages: readonly PaperAiReadingMessage[];
  readonly status: "started" | "delta" | "completed" | "failed" | "canceled";
  readonly sessionId?: string;
  readonly turnId?: string;
  readonly ordinal?: number;
  readonly errorMessage?: string;
}

interface QueuedTurnDraft {
  readonly id: string;
  readonly text: string;
  readonly modelSelection: LlmModelSelection;
}

interface SendMessageOptions {
  readonly queuedDraft?: QueuedTurnDraft;
  readonly restoreOnFailure?: boolean;
  readonly conceptRetrievalTraceId?: string;
}

interface PendingRetrievalSend extends PendingConceptRetrieval {
  readonly requestId: string;
  readonly selection: LlmModelSelection;
  readonly options: SendMessageOptions;
}

interface ActiveTurnRequest {
  readonly clientRequestId: string;
  readonly modelSelection: LlmModelSelection;
  canceled: boolean;
}

type PaperAiReadingBusyState = "idle" | "loading" | "retrieving" | "preparing" | "parsing" | "rendering" | "sending";

function sessionModelSelection(session: PaperAiReadingSessionSummary): LlmModelSelection {
  return {
    providerId: session.providerId,
    modelId: session.modelId,
    ...(session.reasoningEffort === undefined ? {} : { reasoningEffort: session.reasoningEffort }),
  };
}

function isPaperAiReadingStreamEvent(value: unknown): value is PaperAiReadingStreamEvent {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const event = value as Partial<PaperAiReadingStreamEvent>;
  return typeof event.clientRequestId === "string"
    && typeof event.paperId === "string"
    && typeof event.sessionId === "string"
    && typeof event.turnId === "string"
    && typeof event.ordinal === "number"
    && (event.status === "started" || event.status === "delta" || event.status === "completed")
    && (event.messageId === undefined || typeof event.messageId === "string")
    && (event.delta === undefined || typeof event.delta === "string")
    && typeof event.accumulated === "string";
}

export function PaperAiReadingPanel({
  aiGateway: suppliedAiGateway,
  doclingGateway: suppliedDoclingGateway,
  item,
  executeReaderCommand,
  literatureGateway: suppliedLiteratureGateway,
  onHighlightContext,
  onClose,
  hidden = false,
}: PaperAiReadingPanelProps): React.JSX.Element {
  const aiGateway = useMemo(() => suppliedAiGateway ?? getAiAssistanceGateway(), [suppliedAiGateway]);
  const doclingGateway = useMemo(() => suppliedDoclingGateway ?? getDoclingParserGateway(), [suppliedDoclingGateway]);
  const literatureGateway = useMemo(() => suppliedLiteratureGateway ?? getLiteratureGateway(), [suppliedLiteratureGateway]);
  const attachmentIds = useMemo(
    () => item.attachments.filter((attachment) => attachment.mimeType === "application/pdf").map((attachment) => attachment.id),
    [item.attachments],
  );
  const [prepare, setPrepare] = useState<PaperAiReadingPrepareResult>();
  const [session, setSession] = useState<PaperAiReadingSessionDetail>();
  const [sessions, setSessions] = useState<readonly PaperAiReadingSessionSummary[]>([]);
  const [bootstrapOptions, setBootstrapOptions] = useState<PaperReadingBootstrapOptions>();
  const [includeStructuredHistory, setIncludeStructuredHistory] = useState(false);
  const [includeConceptRetrieval, setIncludeConceptRetrieval] = useState(false);
  const [pendingRetrieval, setPendingRetrieval] = useState<PendingRetrievalSend>();
  const [message, setMessage] = useState("");
  const [modelSelection, setModelSelection] = useState<LlmModelSelection>();
  const [busyState, setBusyStateValue] = useState<PaperAiReadingBusyState>("idle");
  const [error, setError] = useState<string>();
  const [streamingTurn, setStreamingTurn] = useState<StreamingTurnState>();
  const [queuedTurns, setQueuedTurns] = useState<readonly QueuedTurnDraft[]>([]);
  const [queueCollapsed, setQueueCollapsed] = useState(false);
  const [interrupting, setInterrupting] = useState(false);
  const [speechInputState, setSpeechInputState] = useState<SpeechInputState>("idle");
  const activeStreamRequestId = useRef<string | undefined>(undefined);
  const activeRetrievalRequestId = useRef<string | undefined>(undefined);
  const pendingStreamEvent = useRef<PaperAiReadingStreamEvent | undefined>(undefined);
  const streamFlushTimer = useRef<number | undefined>(undefined);
  const activeTurnRequest = useRef<ActiveTurnRequest | undefined>(undefined);
  const busyStateRef = useRef<PaperAiReadingBusyState>("idle");
  const queuedTurnsRef = useRef<readonly QueuedTurnDraft[]>([]);
  const sessionRef = useRef<PaperAiReadingSessionDetail | undefined>(undefined);
  const messagesRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    sessionRef.current = session;
  }, [session]);

  function setBusyState(next: PaperAiReadingBusyState): void {
    busyStateRef.current = next;
    setBusyStateValue(next);
  }

  useEffect(() => {
    let cancelled = false;
    async function load(): Promise<void> {
      setBusyState("loading");
      setError(undefined);
      try {
        const [prepared, nextSessions, nextBootstrapOptions] = await Promise.all([
          aiGateway.preparePaperReadingContext(item.id, attachmentIds),
          aiGateway.listPaperReadingSessions(item.id),
          aiGateway.preparePaperReadingBootstrap(item.id, attachmentIds).catch(() => ({
            defaultKind: "fresh_paper" as const,
            unavailableReason: "无法检查结构化阅读历史；仍可建立全新论文会话",
          })),
        ]);
        if (cancelled) return;
        setPrepare(prepared);
        setSessions(nextSessions);
        setBootstrapOptions(nextBootstrapOptions);
        setIncludeStructuredHistory(nextBootstrapOptions.defaultKind === "structured_history");
        if (prepared.session !== undefined) {
          const detail = await aiGateway.getPaperReadingSession(prepared.session.id);
          if (!cancelled) {
            setSession(detail);
            setModelSelection(sessionModelSelection(detail));
            setIncludeConceptRetrieval(detail.conceptRetrievalDefault);
          }
        } else {
          setSession(undefined);
          setModelSelection(prepared.model);
        }
      } catch (reason: unknown) {
        if (!cancelled) setError(aiAssistanceErrorMessage(reason));
      } finally {
        if (!cancelled) setBusyState("idle");
      }
    }
    void load();
    return () => { cancelled = true; };
  }, [aiGateway, attachmentIds, item.id]);

  useEffect(() => {
    if (!isDesktopRuntime()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const applyStreamEvent = (payload: PaperAiReadingStreamEvent): void => {
      if (payload.clientRequestId !== activeStreamRequestId.current) return;
      setStreamingTurn((current) => current?.clientRequestId !== payload.clientRequestId
        ? current
        : {
          ...current,
          messages: payload.messageId === undefined
            ? current.messages
            : upsertAssistantMessage(current.messages, payload.messageId, payload.accumulated),
          status: payload.status,
          sessionId: payload.sessionId,
          turnId: payload.turnId,
          ordinal: payload.ordinal,
        });
    };
    const flushStreamEvent = (): void => {
      streamFlushTimer.current = undefined;
      const payload = pendingStreamEvent.current;
      pendingStreamEvent.current = undefined;
      if (payload !== undefined) applyStreamEvent(payload);
    };
    void listen<unknown>("paper-ai-reading-turn-stream", (event) => {
      const payload = event.payload;
      if (!isPaperAiReadingStreamEvent(payload)
        || payload.paperId !== item.id
        || payload.clientRequestId !== activeStreamRequestId.current) {
        return;
      }
      pendingStreamEvent.current = payload;
      if (payload.status === "completed") {
        if (streamFlushTimer.current !== undefined) window.clearTimeout(streamFlushTimer.current);
        flushStreamEvent();
      } else if (streamFlushTimer.current === undefined) {
        streamFlushTimer.current = window.setTimeout(flushStreamEvent, 24);
      }
    })
      .then((handler) => {
        if (disposed) handler();
        else unlisten = handler;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      if (streamFlushTimer.current !== undefined) window.clearTimeout(streamFlushTimer.current);
      streamFlushTimer.current = undefined;
      pendingStreamEvent.current = undefined;
      unlisten?.();
    };
  }, [item.id]);

  async function refreshSessionSummaries(): Promise<void> {
    const nextSessions = await aiGateway.listPaperReadingSessions(item.id);
    setSessions(nextSessions);
  }

  function startNewSession(): void {
    sessionRef.current = undefined;
    setSession(undefined);
    setModelSelection(prepare?.model);
    setIncludeStructuredHistory(bootstrapOptions?.defaultKind === "structured_history");
    setIncludeConceptRetrieval(false);
    setPendingRetrieval(undefined);
  }

  async function selectSession(sessionId: string): Promise<void> {
    if (busyStateRef.current !== "idle") return;
    setError(undefined);
    if (sessionId === "") {
      startNewSession();
      return;
    }
    setBusyState("loading");
    try {
      const next = await aiGateway.getPaperReadingSession(sessionId);
      sessionRef.current = next;
      setSession(next);
      setModelSelection(sessionModelSelection(next));
      setIncludeConceptRetrieval(next.conceptRetrievalDefault);
    } catch (reason: unknown) {
      setError(aiAssistanceErrorMessage(reason));
    } finally {
      setBusyState("idle");
    }
  }

  async function deleteSession(sessionId: string): Promise<void> {
    if (busyStateRef.current !== "idle" || sessionId === "") return;
    const currentSession = session?.id === sessionId ? session : undefined;
    const title = currentSession?.title.trim() || "AI 阅读会话";
    if (!window.confirm(`删除「${title}」？该对话会从当前列表移除。`)) return;
    setBusyState("loading");
    setError(undefined);
    try {
      const deleted = await aiGateway.deletePaperReadingSession(sessionId);
      if (!deleted) throw new Error("找不到要删除的 AI 阅读对话。");
      const nextSessions = await aiGateway.listPaperReadingSessions(item.id);
      setSessions(nextSessions);
      const nextSession = nextSessions[0];
      sessionRef.current = undefined;
      setSession(undefined);
      if (nextSession !== undefined) {
        const detail = await aiGateway.getPaperReadingSession(nextSession.id);
        sessionRef.current = detail;
        setSession(detail);
        setModelSelection(sessionModelSelection(detail));
        setIncludeConceptRetrieval(detail.conceptRetrievalDefault);
      } else {
        setModelSelection(prepare?.model);
        setIncludeConceptRetrieval(false);
      }
      notify.success("AI 阅读对话已删除");
    } catch (reason: unknown) {
      const errorMessage = aiAssistanceErrorMessage(reason);
      setError(errorMessage);
      notify.error(errorMessage);
    } finally {
      setBusyState("idle");
    }
  }

  function setQueuedTurnList(next: readonly QueuedTurnDraft[]): void {
    queuedTurnsRef.current = next;
    setQueuedTurns(next);
  }

  function scrollConversationToEnd(): void {
    window.requestAnimationFrame(() => {
      const messages = messagesRef.current;
      if (messages !== null) messages.scrollTop = messages.scrollHeight;
    });
  }

  function popNextQueuedTurn(): QueuedTurnDraft | undefined {
    const [next, ...rest] = queuedTurnsRef.current;
    setQueuedTurnList(rest);
    return next;
  }

  async function submitSteeredFollowup(draft: QueuedTurnDraft): Promise<void> {
    const request = activeTurnRequest.current;
    if (request === undefined) {
      setQueuedTurnList([draft, ...queuedTurnsRef.current]);
      return;
    }
    const optimisticMessage: PaperAiReadingMessage = {
      id: `pending-${draft.id}`,
      role: "user",
      content: draft.text,
    };
    setStreamingTurn((current) => current?.clientRequestId === request.clientRequestId
      ? { ...current, messages: [...current.messages, optimisticMessage] }
      : current);
    scrollConversationToEnd();
    try {
      const result = await aiGateway.steerPaperReadingTurn(request.clientRequestId, draft.text);
      if (result.status === "accepted" && result.message !== undefined) {
        setStreamingTurn((current) => current?.clientRequestId === request.clientRequestId
          ? {
            ...current,
            messages: current.messages.map((message) => message.id === optimisticMessage.id
              ? result.message!
              : message),
          }
          : current);
        return;
      }
      removeOptimisticSteer(request.clientRequestId, optimisticMessage.id);
      setQueuedTurnList([draft, ...queuedTurnsRef.current]);
      notify.info(result.status === "unsupported"
        ? "当前模型不支持运行中插入；消息已加入下一轮队列"
        : "当前回答尚未进入可插入阶段；消息已加入下一轮队列");
    } catch (reason: unknown) {
      removeOptimisticSteer(request.clientRequestId, optimisticMessage.id);
      setQueuedTurnList([draft, ...queuedTurnsRef.current]);
      notify.error(`无法插入当前回答，消息已保留在队列：${aiAssistanceErrorMessage(reason)}`);
    }
  }

  function removeOptimisticSteer(clientRequestId: string, messageId: string): void {
    setStreamingTurn((current) => current?.clientRequestId === clientRequestId
      ? { ...current, messages: current.messages.filter((message) => message.id !== messageId) }
      : current);
  }

  function steerQueuedTurn(id: string): void {
    const selected = queuedTurnsRef.current.find((turn) => turn.id === id);
    if (selected === undefined) return;
    setQueuedTurnList(queuedTurnsRef.current.filter((turn) => turn.id !== id));
    void submitSteeredFollowup(selected);
  }

  function editQueuedTurn(id: string): void {
    const index = queuedTurnsRef.current.findIndex((turn) => turn.id === id);
    if (index < 0) return;
    const selected = queuedTurnsRef.current[index];
    if (selected === undefined) return;
    const next = queuedTurnsRef.current.filter((turn) => turn.id !== id);
    const currentText = message.trim();
    if (currentText.length > 0 && modelSelection !== undefined) {
      next.splice(index, 0, {
        id: globalThis.crypto.randomUUID(),
        text: currentText,
        modelSelection,
      });
    }
    setQueuedTurnList(next);
    setMessage(selected.text);
    setModelSelection(selected.modelSelection);
    window.requestAnimationFrame(() => textareaRef.current?.focus());
  }

  function submitComposer(): void {
    if (speechInputState !== "idle") return;
    const text = message.trim();
    if (text.length === 0) return;
    if (modelSelection === undefined) {
      setError("请先选择可用的视觉模型。");
      return;
    }
    const draft: QueuedTurnDraft = {
      id: globalThis.crypto.randomUUID(),
      text,
      modelSelection,
    };
    setMessage("");
    if (busyStateRef.current !== "idle") {
      void submitSteeredFollowup(draft);
      return;
    }
    void beginSend(text, modelSelection, { restoreOnFailure: true });
  }

  function sendNextFollowup(): void {
    const queuedDraft = popNextQueuedTurn();
    if (queuedDraft !== undefined) {
      void beginSend(queuedDraft.text, queuedDraft.modelSelection, { queuedDraft });
    }
  }

  async function beginSend(
    text: string,
    selection: LlmModelSelection,
    options: SendMessageOptions,
  ): Promise<void> {
    if (!includeConceptRetrieval) {
      await sendMessage(text, selection, options);
      return;
    }
    const requestId = globalThis.crypto.randomUUID();
    activeRetrievalRequestId.current = requestId;
    setBusyState("retrieving");
    setError(undefined);
    setPendingRetrieval({ text, selection, options, requestId, isLoading: true });
    try {
      const preview = await aiGateway.previewConceptRetrieval({
        clientRequestId: requestId,
        paperId: item.id,
        query: text,
      });
      if (activeRetrievalRequestId.current !== requestId) return;
      setPendingRetrieval({ text, selection, options, requestId, preview, isLoading: false });
    } catch (reason: unknown) {
      if (activeRetrievalRequestId.current !== requestId) return;
      restorePendingMessage(text, options);
      setPendingRetrieval(undefined);
      const errorMessage = aiAssistanceErrorMessage(reason);
      if (errorMessage !== "概念网络检索已取消") {
        setError(errorMessage);
        notify.error(errorMessage);
      }
    } finally {
      if (activeRetrievalRequestId.current === requestId) {
        activeRetrievalRequestId.current = undefined;
        setBusyState("idle");
      }
    }
  }

  function restorePendingMessage(text: string, options: SendMessageOptions): void {
    if (options.queuedDraft !== undefined) {
      setQueuedTurnList([options.queuedDraft, ...queuedTurnsRef.current]);
    } else {
      setMessage((current) => current.trim().length === 0 ? text : current);
    }
  }

  function confirmConceptRetrieval(): void {
    const pending = pendingRetrieval;
    if (pending?.preview === undefined || pending.isLoading) return;
    setPendingRetrieval(undefined);
    void sendMessage(pending.text, pending.selection, {
      ...pending.options,
      conceptRetrievalTraceId: pending.preview.traceId,
    });
  }

  function cancelConceptRetrieval(): void {
    const pending = pendingRetrieval;
    if (pending === undefined) return;
    activeRetrievalRequestId.current = undefined;
    setPendingRetrieval(undefined);
    setBusyState("idle");
    restorePendingMessage(pending.text, pending.options);
    if (pending.isLoading) {
      void aiGateway.cancelConceptRetrievalPreview(pending.requestId).catch(() => undefined);
    }
  }

  async function refreshAfterCancellation(): Promise<void> {
    const summaries = await aiGateway.listPaperReadingSessions(item.id);
    setSessions(summaries);
    const sessionId = sessionRef.current?.id ?? summaries[0]?.id;
    if (sessionId === undefined) return;
    const detail = await aiGateway.getPaperReadingSession(sessionId);
    sessionRef.current = detail;
    setSession(detail);
    setModelSelection(sessionModelSelection(detail));
  }

  async function interruptCurrentTurn(): Promise<void> {
    if (activeRetrievalRequestId.current !== undefined) {
      cancelConceptRetrieval();
      return;
    }
    const request = activeTurnRequest.current;
    if (request === undefined || interrupting) return;
    request.canceled = true;
    setInterrupting(true);
    setStreamingTurn((current) => current?.clientRequestId === request.clientRequestId
      ? { ...current, status: "canceled", errorMessage: "正在中断…" }
      : current);
    try {
      if (busyStateRef.current === "sending") {
        const result = await aiGateway.cancelPaperReadingTurn(request.clientRequestId);
        if (!result.accepted) throw new Error("当前回答尚未进入可中断阶段，请稍后重试。");
        if (result.resubmitMessages.length > 0) {
          const recovered = result.resubmitMessages.map((message) => ({
            id: globalThis.crypto.randomUUID(),
            text: message.content,
            modelSelection: request.modelSelection,
          }));
          setQueuedTurnList([...recovered, ...queuedTurnsRef.current]);
        }
      }
    } catch (reason: unknown) {
      request.canceled = false;
      setInterrupting(false);
      setStreamingTurn((current) => {
        if (current?.clientRequestId !== request.clientRequestId) return current;
        const { errorMessage: _errorMessage, ...activeTurn } = current;
        return {
          ...activeTurn,
          status: hasAssistantMessage(current.messages) ? "delta" : "started",
        };
      });
      setError(`无法中断当前回答：${aiAssistanceErrorMessage(reason)}`);
    }
  }

  function ensureNotCanceled(request: ActiveTurnRequest): void {
    if (request.canceled) throw new Error("AI 运行已取消");
  }

  async function sendMessage(
    text: string,
    selection: LlmModelSelection | undefined,
    options: SendMessageOptions = {},
  ): Promise<void> {
    if (text.length === 0 || busyStateRef.current !== "idle") return;
    if (selection === undefined) {
      setError("请先选择可用的视觉模型。");
      return;
    }
    setError(undefined);
    const clientRequestId = globalThis.crypto.randomUUID();
    const turnRequest: ActiveTurnRequest = {
      clientRequestId,
      modelSelection: selection,
      canceled: false,
    };
    activeTurnRequest.current = turnRequest;
    activeStreamRequestId.current = clientRequestId;
    let shouldContinueFollowups = false;
    setStreamingTurn({
      clientRequestId,
      messages: [{
        id: `${clientRequestId}-user-0`,
        role: "user",
        content: text,
      }],
      status: "started",
    });
    try {
      setBusyState("preparing");
      let prepared = await aiGateway.preparePaperReadingContext(item.id, attachmentIds);
      ensureNotCanceled(turnRequest);
      const missing = prepared.attachmentStatuses.filter((status) => status.parseStatus !== "available");
      if (missing.length > 0) {
        setBusyState("parsing");
        for (const status of missing) {
          await doclingGateway.parseAttachment(item.id, status.attachmentId);
          ensureNotCanceled(turnRequest);
        }
        prepared = await aiGateway.preparePaperReadingContext(item.id, attachmentIds);
        ensureNotCanceled(turnRequest);
      }
      const stillMissing = prepared.attachmentStatuses.filter((status) => status.parseStatus !== "available");
      if (stillMissing.length > 0) {
        throw new Error("部分 PDF 尚未完成 Docling 结构化解析，无法发送给 AI。");
      }
      setPrepare(prepared);
      const sessionId = sessionRef.current?.id;
      const imageAssets: Array<{ readonly contextItemId: string; readonly dataUrl: string }> = [];
      if (sessionId === undefined) {
        setBusyState("rendering");
        const attachmentBytes = new Map<string, Uint8Array>();
        for (const request of prepared.imageRequests) {
          const region = request.regions[0];
          if (region === undefined) throw new Error(`结构化图片块 ${request.contextItemId} 没有可截图区域`);
          let bytes = attachmentBytes.get(request.attachmentId);
          if (bytes === undefined) {
            bytes = await literatureGateway.readAttachment(request.attachmentId);
            attachmentBytes.set(request.attachmentId, bytes);
          }
          const dataUrl = await renderPdfRegionPng(bytes, region.pageIndex, region.rect);
          ensureNotCanceled(turnRequest);
          imageAssets.push({ contextItemId: request.contextItemId, dataUrl });
        }
      }
      setBusyState("sending");
      ensureNotCanceled(turnRequest);
      const next = await aiGateway.startPaperReadingTurn({
        clientRequestId,
        ...(sessionId === undefined ? {} : { sessionId }),
        ...(sessionId !== undefined ? {} : includeStructuredHistory
          && bootstrapOptions?.structuredHistory !== undefined
          ? { bootstrapKind: "structured_history" as const,
              sourceWorkflowId: bootstrapOptions.structuredHistory.workflowId }
          : { bootstrapKind: "fresh_paper" as const }),
        paperId: item.id,
        attachmentIds,
        userMessage: text,
        ...(options.conceptRetrievalTraceId === undefined
          ? {} : { conceptRetrievalTraceId: options.conceptRetrievalTraceId }),
        modelSelection: selection,
        imageAssets,
      });
      sessionRef.current = next;
      setSession(next);
      setModelSelection(sessionModelSelection(next));
      activeStreamRequestId.current = undefined;
      setStreamingTurn(undefined);
      void refreshSessionSummaries().catch(() => undefined);
      notify.success("AI 阅读回答已生成");
      shouldContinueFollowups = true;
    } catch (reason: unknown) {
      const errorMessage = aiAssistanceErrorMessage(reason);
      const canceled = turnRequest.canceled || errorMessage === "AI 运行已取消";
      if (canceled) {
        setError(undefined);
        setStreamingTurn(undefined);
        await refreshAfterCancellation().catch(() => undefined);
        notify.info("已中断当前 AI 回答；待发送消息仍保留在队列中");
      } else {
        setError(errorMessage);
        if (options.queuedDraft !== undefined) {
          setQueuedTurnList([options.queuedDraft, ...queuedTurnsRef.current]);
        } else if (options.restoreOnFailure) {
          setMessage((current) => current.trim().length === 0 ? text : current);
        }
        setStreamingTurn((current) => current?.clientRequestId === clientRequestId
          ? { ...current, status: "failed", errorMessage }
          : current);
        notify.error(errorMessage);
      }
    } finally {
      if (activeStreamRequestId.current === clientRequestId) activeStreamRequestId.current = undefined;
      if (activeTurnRequest.current === turnRequest) activeTurnRequest.current = undefined;
      setInterrupting(false);
      setBusyState("idle");
      if (shouldContinueFollowups) window.setTimeout(sendNextFollowup, 0);
    }
  }

  function jumpToContext(itemId: string): void {
    const contextItem = prepare?.contextItems.find((candidate) => candidate.id === itemId);
    const region = contextItem?.regions[0];
    if (contextItem === undefined || region === undefined) {
      notify.warning(`无法定位结构化块 ${itemId}`);
      return;
    }
    executeReaderCommand("reader.navigate", { pageIndex: region.pageIndex });
    onHighlightContext?.(itemId, contextItem.regions);
  }

  const statusText = prepare === undefined
    ? "正在读取结构化上下文…"
    : `${prepare.attachmentStatuses.length} 个 PDF · ${prepare.textItemCount} 文本块 · ${prepare.tableItemCount} 表格 · ${prepare.pictureItemCount} 图片`;
  const selectedSessionId = session?.id ?? "";
  const usageTurn = latestUsageTurn(session?.turns ?? []);
  const hasComposerMessage = message.trim().length > 0;
  const canSubmitMessage = hasComposerMessage
    && modelSelection !== undefined
    && busyState !== "loading"
    && pendingRetrieval === undefined
    && speechInputState === "idle";
  const canQueueWhileBusy = busyState !== "idle" && busyState !== "loading";
  const primaryActionStops = canQueueWhileBusy && !hasComposerMessage;
  const composerStatus = [
    busyState === "idle" ? "" : busyStateLabel(busyState),
    speechInputStateLabel(speechInputState),
  ].filter((value) => value !== "").join(" · ");

  return (
    <aside aria-label="AI 文献阅读" className="paper-ai-reading-panel" hidden={hidden}>
      <header>
        <div>
          <strong>AI 文献阅读</strong>
          <span>{statusText}</span>
        </div>
        <ControlHint content={busyState === "idle" ? "关闭 AI 阅读" : "隐藏；回答会在后台继续生成"}><button
          aria-label={busyState === "idle" ? "关闭 AI 阅读" : "隐藏 AI 阅读（回答继续生成）"}
          onClick={onClose}
          type="button"
        ><X /></button></ControlHint>
      </header>
      {prepare !== undefined && <div className="paper-ai-reading-status">
        <span>解析：{prepare.attachmentStatuses.map((status) => `${status.displayName} ${status.parseStatus === "available" ? "可用" : "待解析"}`).join("；")}</span>
        {session !== undefined && <PaperReadingBootstrapBadge session={session} />}
      </div>}
      <div className="paper-ai-reading-session-bar">
        <label>
          会话
          <select disabled={busyState !== "idle"} onChange={(event) => void selectSession(event.target.value)} value={selectedSessionId}>
            <option value="">新会话</option>
            {sessions.map((item) => (
              <option key={item.id} value={item.id}>{sessionLabel(item)}</option>
            ))}
          </select>
        </label>
        <div className="paper-ai-reading-session-actions">
          <ControlHint content="删除当前对话"><button
            aria-label="删除当前 AI 阅读对话"
            disabled={busyState !== "idle" || selectedSessionId === ""}
            onClick={() => void deleteSession(selectedSessionId)}
            type="button"
          >
            <Trash2 />
            <span>删除</span>
          </button></ControlHint>
          <button disabled={busyState !== "idle" || selectedSessionId === ""} onClick={startNewSession} type="button">新会话</button>
        </div>
      </div>
      {error !== undefined && <p className="paper-ai-reading-error">{error}</p>}
      {session === undefined && <PaperReadingBootstrapControl checked={includeStructuredHistory}
        disabled={busyState !== "idle"}
        onChange={setIncludeStructuredHistory}
        {...(bootstrapOptions === undefined ? {} : { options: bootstrapOptions })}
        {...(modelSelection === undefined ? {} : { currentModel: modelSelection })} />}
      <div className="paper-ai-reading-messages" ref={messagesRef}>
        {session === undefined && <p className="empty">这是新的 AI 阅读会话。首次发送会冻结当前 PDF 和解析版本；历史会话可从上方切换。</p>}
        {session?.turns.length === 0 && <p className="empty">直接提问。系统会先解析 PDF，再把带 ID 的文本、表格和图片截图发送给视觉模型。</p>}
        {session?.turns.map((turn, index, turns) => <article key={turn.id}>
          {turn.messages.map((item) => item.role === "user"
            ? <p className="paper-ai-reading-user" key={item.id}>{item.content}</p>
            : <PaperAiReadingMarkdown key={item.id} onJump={jumpToContext} value={item.content} />)}
          {!hasAssistantMessage(turn.messages) && <p className="paper-ai-reading-error">{turn.errorMessage ?? "回答生成失败"}</p>}
          <span className="paper-ai-reading-turn-model">{turn.providerId} · {turn.modelId}{turn.reasoningEffort === undefined ? "" : ` · ${turn.reasoningEffort}`}</span>
          {turn.conceptRetrievalTraceId !== undefined && <PaperConceptRetrievalTrace
            aiGateway={aiGateway}
            traceId={turn.conceptRetrievalTraceId}
          />}
          <PaperAiReadingDebugData aiGateway={aiGateway} turn={turn} turns={turns.slice(0, index + 1)} />
        </article>)}
        {streamingTurn !== undefined && <article aria-live="polite">
          {streamingTurn.messages.map((item, index, items) => item.role === "user"
            ? <p className="paper-ai-reading-user" key={item.id}>{item.content}</p>
            : <PaperAiReadingMarkdown
              key={item.id}
              onJump={jumpToContext}
              streaming={(streamingTurn.status === "started" || streamingTurn.status === "delta")
                && index === items.length - 1}
              value={item.content}
            />)}
          {(streamingTurn.messages.length === 0
            || streamingTurn.messages[streamingTurn.messages.length - 1]?.role === "user")
            && <p className={streamingTurn.status === "failed" ? "paper-ai-reading-error" : "empty"}>
              {streamingTurn.errorMessage ?? streamingStatusText(busyState, streamingTurn.status, streamingTurn.messages)}
            </p>}
          <details className="paper-ai-reading-raw">
            <summary>查看流式数据</summary>
            <pre>{JSON.stringify(streamingTurnData(streamingTurn), null, 2)}</pre>
          </details>
        </article>}
      </div>
      <footer className="paper-ai-reading-composer" data-busy={canQueueWhileBusy ? "true" : "false"}>
        {queuedTurns.length > 0 && <div className="paper-ai-reading-followups" aria-label="待发送消息">
          <button
            aria-expanded={!queueCollapsed}
            className="paper-ai-reading-followups-toggle"
            onClick={() => setQueueCollapsed((value) => !value)}
            type="button"
          >
            <strong>{queuedTurns.length} 条待发送</strong>
            {queueCollapsed && <span>{queuedTurns[0]?.text}</span>}
            <ChevronDown aria-hidden="true" />
          </button>
          {!queueCollapsed && <ol>
            {queuedTurns.map((turn) => (
              <li key={turn.id}>
                <span>{queuedTurns[0]?.id === turn.id ? "下一条" : "待发送"}</span>
                <p>{turn.text}</p>
                <div>
                  <ControlHint content="插入当前回答"><button
                    aria-label={`插入当前回答：${turn.text}`}
                    onClick={() => steerQueuedTurn(turn.id)}
                    type="button"
                  ><CornerUpRight aria-hidden="true" /></button></ControlHint>
                  <ControlHint content="返回编辑"><button
                    aria-label={`返回编辑待发送消息：${turn.text}`}
                    onClick={() => editQueuedTurn(turn.id)}
                    type="button"
                  ><Pencil aria-hidden="true" /></button></ControlHint>
                </div>
              </li>
            ))}
          </ol>}
        </div>}
        <PaperConceptRetrievalControl
          checked={includeConceptRetrieval}
          disabled={busyState !== "idle" || pendingRetrieval !== undefined}
          onCancel={cancelConceptRetrieval}
          onChange={setIncludeConceptRetrieval}
          onConfirm={confirmConceptRetrieval}
          {...(pendingRetrieval === undefined ? {} : { pending: pendingRetrieval })}
        />
        <textarea
          aria-label="向 AI 提问当前文献"
          disabled={busyState === "loading"}
          onChange={(event) => setMessage(event.target.value)}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing || event.keyCode === 229) return;
            if (event.key === "Escape" && busyStateRef.current !== "idle") {
              event.preventDefault();
              void interruptCurrentTurn();
              return;
            }
            if (event.key !== "Enter" || event.shiftKey) return;
            event.preventDefault();
            if (speechInputState === "idle") submitComposer();
          }}
          placeholder="询问这篇论文；回答会尽量引用结构化块 ID，例如 [P1-T3]"
          rows={4}
          ref={textareaRef}
          value={message}
        />
        <div className="paper-ai-reading-composer-actions">
          <span aria-live="polite" className="paper-ai-reading-composer-status">{composerStatus}</span>
          <div className="paper-ai-reading-composer-tools">
            <PaperAiReadingContextIndicator turn={usageTurn} />
            <PaperAiReadingModelControl
              disabled={busyState !== "idle"}
              gateway={aiGateway}
              onSelection={setModelSelection}
              selection={modelSelection}
            />
            <PaperAiReadingSpeechInput
              disabled={busyState === "loading" || hidden}
              onChange={setMessage}
              onStateChange={setSpeechInputState}
              value={message}
            />
            <ControlHint content={primaryActionStops
                ? "中断当前回答（Esc）"
                : canQueueWhileBusy ? "插入当前回答（Enter）" : "发送（Enter）"}><button
              aria-label={primaryActionStops ? "中断当前回答" : "发送"}
              className="paper-ai-reading-send"
              data-action={primaryActionStops ? "stop" : "send"}
              disabled={primaryActionStops ? interrupting : !canSubmitMessage}
              onClick={() => {
                if (primaryActionStops) void interruptCurrentTurn();
                else submitComposer();
              }}
              type="button"
            >
              {primaryActionStops
                ? interrupting
                  ? <Loader2 className="spin" aria-hidden="true" />
                  : <Square aria-hidden="true" />
                : busyState === "loading"
                  ? <Loader2 className="spin" aria-hidden="true" />
                  : <ArrowUp aria-hidden="true" />}
            </button></ControlHint>
          </div>
        </div>
      </footer>
    </aside>
  );
}

function busyStateLabel(
  busyState: "loading" | "retrieving" | "preparing" | "parsing" | "rendering" | "sending",
): string {
  if (busyState === "loading") return "正在读取会话";
  if (busyState === "retrieving") return "正在检索概念网络";
  if (busyState === "preparing") return "正在准备上下文";
  if (busyState === "parsing") return "正在解析 PDF";
  if (busyState === "rendering") return "正在生成图片";
  return "正在生成回答";
}

function hasAssistantMessage(messages: readonly PaperAiReadingMessage[]): boolean {
  return messages.some((message) => message.role === "assistant" && message.content.length > 0);
}

function upsertAssistantMessage(
  messages: readonly PaperAiReadingMessage[],
  messageId: string,
  content: string,
): readonly PaperAiReadingMessage[] {
  const index = messages.findIndex((message) => message.id === messageId);
  if (index < 0) return [...messages, { id: messageId, role: "assistant", content }];
  const current = messages[index];
  if (current?.role !== "assistant" || current.content === content) return messages;
  const next = [...messages];
  next[index] = { ...current, content };
  return next;
}

function streamingStatusText(
  busyState: "idle" | "loading" | "retrieving" | "preparing" | "parsing" | "rendering" | "sending",
  streamStatus: StreamingTurnState["status"],
  messages: readonly PaperAiReadingMessage[],
): string {
  if (streamStatus === "completed") return "正在保存 AI 回答…";
  if (busyState === "preparing") return "正在准备结构化上下文…";
  if (busyState === "retrieving") return "正在检索概念网络…";
  if (busyState === "parsing") return "正在解析 PDF…";
  if (busyState === "rendering") return "正在生成图片截图…";
  if (busyState === "sending") return hasAssistantMessage(messages)
    ? "等待模型处理插入消息…"
    : "等待模型返回首个片段…";
  return "正在启动 AI 阅读…";
}

function streamingTurnData(turn: StreamingTurnState): Record<string, unknown> {
  return {
    clientRequestId: turn.clientRequestId,
    sessionId: turn.sessionId,
    turnId: turn.turnId,
    ordinal: turn.ordinal,
    status: turn.status,
    messages: turn.messages,
    errorMessage: turn.errorMessage,
    retention: "transient_stream_event_only; final_turn_is_persisted_after_provider_completion",
  };
}

function sessionLabel(session: PaperAiReadingSessionSummary): string {
  const title = session.title.trim().length === 0 ? "AI 阅读会话" : session.title.trim();
  return `${title} · ${session.updatedAt}`;
}

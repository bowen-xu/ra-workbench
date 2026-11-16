import type { MutableRefObject } from "react";
import { getRuntimeEnvironment } from "../../../../../../apps/web/src/core/public/runtimeEnvironment";
import { freezeNewPreviewOwnerIntent, NATIVE_PREVIEW_OWNER_CAPABILITY } from "../../../../../../apps/web/src/core/public/previewOwner";
import { reserveBoundConversation } from "../../../../../../apps/web/src/core/public/agentConversations/boundConversationCreationPending";
import type { AgentConversationSummary, AgentModelSelection, AgentUserInput } from "@research-assistant/core/agent";
import type { LlmModelSelection } from "@research-assistant/core/ai";
import type { LiteratureItem } from "@research-assistant/core/literature";
import { agentPaperContextGateway } from "../../../../../../apps/web/src/core/public/agent";
import { aiAssistanceErrorMessage, type AiAssistanceGateway } from "../../../../../../apps/web/src/core/public/aiAssistance";
import type { DoclingParserGateway } from "../../../../../../apps/web/src/core/public/docling";
import type { LiteratureGateway } from "../../../../../../../plugins/ra-literature-library/src/services/literatureGateway";
import { notify } from "../../../../../../apps/web/src/core/public/feedback";
import { readComposerCommand } from "../../../../../../apps/web/src/core/public/agentSubmission/agentComposerCommands";
import { dispatchQueuedAgentCommand, queuedCommandTransfer } from "../../../../../../apps/web/src/core/public/agentSubmission/dispatchQueuedAgentCommand";
import { queueDraftEdit } from "../../../../../../apps/web/src/core/public/agentSubmission/queueComposerDraft";
import type { useConversationDraft } from "../../../../../../apps/web/src/core/public/agentSubmission/AgentDraftProvider";
import type { useAgentComposerInput } from "../../../../../../apps/web/src/core/public/agentSubmission/useAgentComposerInput";
import type { useAgentQueueAutoPrepare } from "../../../../../../apps/web/src/core/public/agentSubmission/useAgentQueueAutoPrepare";
import type { usePaperSteering } from "./usePaperSteering";
import type { usePaperInputQueue } from "./usePaperInputQueue";
import type { SpeechInputState } from "../../../../../../apps/web/src/core/public/speech";
import { renderPdfRegionPng } from "../../../../../../../plugins/ra-literature-library/src/features/reader/pdf/pdfRegionScreenshot";
import { beginPaperOwnerWork, endPaperOwnerWork, paperConversationOwner, setPaperOwnerPendingRetrieval, setPaperOwnerPendingSubmission,
  ensurePaperPreparation, type PaperConversationOwner, type PendingPaperSubmission, type PendingRetrievalSend } from "./paperConversationOwners";
import type { ConversationDraftStore } from "../../../../../../apps/web/src/core/public/agentSubmission/conversationDraftStore";

type Queue = ReturnType<typeof usePaperInputQueue>;
interface PaperQuestionSubmissionOptions {
  readonly item: Pick<LiteratureItem, "id">; readonly conversationId: string | undefined; readonly message: string;
  readonly inputResources: ReturnType<typeof useAgentComposerInput>; readonly draftResources: ReturnType<typeof useConversationDraft>[3];
  readonly busy: boolean; readonly workBusy: boolean; readonly selectedConversation: AgentConversationSummary | undefined;
  readonly paperSteering: ReturnType<typeof usePaperSteering>; readonly speechInputState: SpeechInputState;
  readonly pendingSubmission: PendingPaperSubmission | undefined; readonly pendingRetrieval: PendingRetrievalSend | undefined;
  readonly activeTurn: { readonly conversationId: string; readonly turnId: string } | undefined; readonly includeConceptRetrieval: boolean;
  readonly aiGateway: AiAssistanceGateway; readonly doclingGateway: DoclingParserGateway; readonly literatureGateway: Pick<LiteratureGateway, "readAttachment">;
  readonly attachmentIds: readonly string[]; readonly modelSelection: LlmModelSelection | undefined;
  readonly paperEpochRef: MutableRefObject<number>; readonly paperOwnerRef: MutableRefObject<PaperConversationOwner | null>;
  readonly selectionRevision: MutableRefObject<number>; readonly activeRetrievalRequestId: MutableRefObject<string | undefined>;
  readonly submissionGenerationRef: MutableRefObject<number>; readonly identityRef: MutableRefObject<{ paperId: string; conversationId: string | undefined }>;
  readonly queueController: Queue["controller"]; readonly queueControllerState: Queue["state"]; readonly retainTransfer: Queue["retainTransfer"];
  readonly queueState: NonNullable<Queue["state"]>["queue"] | undefined; readonly queuedInputId: string | undefined;
  readonly autoPrepare: Pick<ReturnType<typeof useAgentQueueAutoPrepare>, "markQueued">;
  readonly captureDraftAcknowledgment: ReturnType<typeof useConversationDraft>[2];
  readonly draftStore: ConversationDraftStore;
  readonly createConversation: (epoch: number, id?: string) => Promise<string | undefined>;
  readonly setBusy: (value: boolean) => void; readonly setError: (value: string | undefined) => void; readonly setMessage: (value: string) => void;
  readonly setPendingRetrieval: (value: PendingRetrievalSend | undefined) => void;
  readonly setContext: (value: Awaited<ReturnType<typeof agentPaperContextGateway.preview>>) => void;
}

function agentModel(selection: LlmModelSelection | undefined): AgentModelSelection | undefined {
  return selection === undefined ? undefined : {
    providerId: selection.providerId,
    modelId: selection.modelId,
    reasoningEffort: selection.reasoningEffort ?? null,
    ...(selection.serviceTier ? { serviceTier: selection.serviceTier } : {}),
  };
}

/** The existing Reader send/source preparation capability. Ports, owners and
 * lifecycle fences stay application-owned; this does not run a model loop. */
export function createPaperQuestionSubmission(options: PaperQuestionSubmissionOptions) {
  const { item, conversationId, message, inputResources, draftResources, busy, workBusy, selectedConversation, paperSteering,
    speechInputState, pendingSubmission, pendingRetrieval, activeTurn, includeConceptRetrieval, aiGateway, doclingGateway,
    literatureGateway, attachmentIds, modelSelection, paperEpochRef, paperOwnerRef, selectionRevision, activeRetrievalRequestId,
    submissionGenerationRef, identityRef, queueController, queueControllerState, queueState, retainTransfer, queuedInputId,
    autoPrepare, captureDraftAcknowledgment, draftStore, createConversation, setBusy, setError, setMessage, setPendingRetrieval, setContext } = options;
  async function send(retrievalTraceId?: string, explicitText?: string, queuedInputIdOverride?: string, frozenInput?: readonly AgentUserInput[],
    frozenSubmission?: {readonly conversationId:string; readonly requestId:string}): Promise<void> {
    if (explicitText === undefined && !queuedInputIdOverride && inputResources.consumeCommand()) return;
    const text = (explicitText ?? message).trim();
    if (inputResources.commandPending) {
      if (!queuedInputIdOverride && text && !workBusy && !selectedConversation?.archived && !paperSteering.unresolved && speechInputState === "idle" && !pendingSubmission && !pendingRetrieval) {
        await enqueue(text);
      }
      return;
    }
    if (queuedInputIdOverride && readComposerCommand(text)) {
      if (busy || paperSteering.unresolved || selectedConversation?.archived) return;
      await dispatchQueuedAgentCommand({inputId: queuedInputIdOverride, text, running: activeTurn !== undefined, execution: {compact: inputResources.executeCommand},
        transfer: () => queuedCommandTransfer(queueController, queuedInputIdOverride, text, () => retainTransfer({type: "remove", inputId: queuedInputIdOverride}))});
      return;
    }
    if (!queuedInputIdOverride && explicitText === undefined && draftResources.draft().queuePlacement) {await enqueue(text); return;}
    const userInput = frozenInput ?? (queuedInputIdOverride ? queueState?.inputs.find(input=>input.inputId===queuedInputIdOverride)?.userInput ?? [] : draftResources.input(text));
    const selectionAtStart = selectionRevision.current;
    if (selectedConversation?.archived) {
      notify.info("此对话已归档，请先恢复后继续发送。");
      return;
    }
    if (!text || busy || paperSteering.unresolved || speechInputState !== "idle" || pendingSubmission !== undefined
      || (pendingRetrieval !== undefined && retrievalTraceId === undefined)) return;
    const epoch = paperEpochRef.current;
    const owner=paperOwnerRef.current;
    if (activeTurn !== undefined) {
      if (paperSteering.unresolved) return;
      setError(undefined);
      const accepted = await paperSteering.submit(text,{userInput});
      if (!accepted && paperEpochRef.current === epoch && selectionRevision.current === selectionAtStart && paperSteering.state.phase === "rejected") {
        setError(paperSteering.state.status);
      }
      return;
    }
    const runtime = getRuntimeEnvironment();
    if (!frozenSubmission && runtime.nativeAgentPreview?.bootstrap.supported.includes(NATIVE_PREVIEW_OWNER_CAPABILITY)) {
      setBusy(true);
      try {
        const ownerConversationId = conversationId ?? reserveBoundConversation({kind:"paper", resourceId:item.id}, undefined, runtime.storage);
        const requestId = crypto.randomUUID();
        await freezeNewPreviewOwnerIntent(runtime.nativeAgentPreview, runtime.storage, ownerConversationId, requestId);
        frozenSubmission = {conversationId:ownerConversationId, requestId};
      } catch (cause) {notify.error(`无法保存预览请求归属，草稿已保留。（${cause instanceof Error ? cause.message : "agent_preview_owner_intent_failed"}）`); return;}
      finally {if (paperEpochRef.current === epoch) setBusy(false);}
      if (paperEpochRef.current !== epoch || selectionRevision.current !== selectionAtStart) return;
    }
    if (includeConceptRetrieval && retrievalTraceId === undefined) {
      const submissionOwner = frozenSubmission;
      const requestId = globalThis.crypto.randomUUID();
      if (owner) beginPaperOwnerWork(owner);
      activeRetrievalRequestId.current = requestId;
      if (owner) setPaperOwnerPendingRetrieval(owner,{text,requestId,userInput,...(submissionOwner ? {submissionOwner} : {}),isLoading:true}); else setPendingRetrieval({ text, requestId,userInput,...(submissionOwner ? {submissionOwner} : {}), isLoading: true });
      setError(undefined);
      try {
        const preview = await aiGateway.previewConceptRetrieval({
          clientRequestId: requestId,
          paperId: item.id,
          query: text,
        });
        if (paperEpochRef.current !== epoch || (owner ? owner.pendingRetrieval?.requestId !== requestId : activeRetrievalRequestId.current !== requestId)) return;
        if (owner) setPaperOwnerPendingRetrieval(owner,{text,requestId,userInput,...(submissionOwner ? {submissionOwner} : {}),preview,isLoading:false}); else setPendingRetrieval({ text, requestId,userInput,...(submissionOwner ? {submissionOwner} : {}), preview, isLoading: false });
      } catch (reason: unknown) {
        if (paperEpochRef.current !== epoch || (owner ? owner.pendingRetrieval?.requestId !== requestId : activeRetrievalRequestId.current !== requestId)) return;
        if (owner) setPaperOwnerPendingRetrieval(owner,undefined); else setPendingRetrieval(undefined);
        if (selectionRevision.current !== selectionAtStart) return;
        const message = aiAssistanceErrorMessage(reason);
        if (message !== "概念网络检索已取消") {
          setError(message);
          notify.error(message);
        }
      } finally {
        if (activeRetrievalRequestId.current === requestId) activeRetrievalRequestId.current = undefined;
        if (owner) endPaperOwnerWork(owner.key,owner);
      }
      return;
    }
    setBusy(true); setError(undefined);
    const draftAcknowledgment = queuedInputIdOverride ? null : draftResources.acknowledgmentToken(text);
    const queued = queuedInputIdOverride ?? queuedInputId;
    const liveQueue = queued ? queueController.current?.getSnapshot() : undefined;
    const queueInput = queued ? liveQueue?.queue?.inputs.find(input => input.inputId === queued) : undefined;
    let stage = "创建会话";
    const generationForUi = submissionGenerationRef.current;
    let submissionOwner: PaperConversationOwner | undefined;
    try {
      if (queued && (!queueInput || !liveQueue || liveQueue.loading || liveQueue.editing || liveQueue.uncertain
        || liveQueue.queue?.paperId !== item.id || liveQueue.queue.conversationId !== conversationId)) throw new Error("队列消息状态已变化，请刷新后重试。");
      const originalQueueInput = queueInput ? structuredClone(queueInput) : undefined;
      const originalQueueRevision = liveQueue?.queue?.revision;
      const id = conversationId ?? await createConversation(epoch, frozenSubmission?.conversationId);
      if (id === undefined || paperEpochRef.current !== epoch) return;
      if (frozenSubmission && frozenSubmission.conversationId !== id) throw new Error("agent_preview_owner_intent_conflict");
      const requestId = frozenSubmission?.requestId ?? globalThis.crypto.randomUUID();
      if (!frozenSubmission) await freezeNewPreviewOwnerIntent(runtime.nativeAgentPreview, runtime.storage, id, requestId);
      if (paperEpochRef.current !== epoch) return;
      submissionOwner=paperConversationOwner(item.id,id);beginPaperOwnerWork(submissionOwner);
      const generation = submissionGenerationRef.current;
      stage = "准备论文上下文";
      let prepared: Awaited<ReturnType<AiAssistanceGateway["preparePaperReadingContext"]>>;
      try {
        prepared = await aiGateway.preparePaperReadingContext(item.id, attachmentIds);
      } catch (reason: unknown) {
        // The legacy reading context is only a compatibility hint for model
        // selection and image requests. Shared Agent context preparation owns
        // the authoritative snapshot and must remain usable when an old
        // session or parser row cannot be decoded.
        if (submissionGenerationRef.current === generation) {
          notify.warning(`旧阅读上下文不可用，已改用共享 Agent 上下文：${aiAssistanceErrorMessage(reason)}`);
        }
        prepared = {
          attachmentStatuses: [],
          contextItems: [],
          imageRequests: [],
          textItemCount: 0,
          tableItemCount: 0,
          pictureItemCount: 0,
        };
      }
      if (paperEpochRef.current !== epoch) return;
      const missing = prepared.attachmentStatuses.filter((status) => status.parseStatus !== "available");
      for (const status of missing) {
        await doclingGateway.parseAttachment(item.id, status.attachmentId);
        if (paperEpochRef.current !== epoch) return;
      }
      if (missing.length > 0) prepared = await aiGateway.preparePaperReadingContext(item.id, attachmentIds);
      if (paperEpochRef.current !== epoch) return;
      const model = agentModel(modelSelection ?? prepared.model);
      const frozenModel = model === undefined ? undefined : { ...model };
      const imageAssets: Array<{ readonly contextItemId: string; readonly dataUrl: string }> = [];
      if (prepared.imageRequests.length > 0) {
        const attachmentBytes = new Map<string, Uint8Array>();
        for (const request of prepared.imageRequests) {
          const region = request.regions[0];
          if (region === undefined) throw new Error(`结构化图片块 ${request.contextItemId} 没有可截图区域`);
          let bytes = attachmentBytes.get(request.attachmentId);
          if (bytes === undefined) {
            bytes = await literatureGateway.readAttachment(request.attachmentId);
            if (paperEpochRef.current !== epoch) return;
            attachmentBytes.set(request.attachmentId, bytes);
          }
          imageAssets.push({ contextItemId: request.contextItemId, dataUrl: await renderPdfRegionPng(bytes, region.pageIndex, region.rect) });
          if (paperEpochRef.current !== epoch) return;
        }
      }
      stage = "冻结共享论文快照";
      const snapshot = await agentPaperContextGateway.prepare({ paperId: item.id, attachmentIds, ...(imageAssets.length === 0 ? {} : { imageAssets }) });
      if (paperEpochRef.current !== epoch) return;
      stage = "读取共享论文上下文";
      const manifest = await agentPaperContextGateway.preview({ paperId: item.id, snapshotId: snapshot.snapshotId, expectedContextHash: snapshot.contextHash });
      if (paperEpochRef.current !== epoch) return;
      if (generation === submissionGenerationRef.current) setContext(manifest);
      stage = "准备 Agent 请求";
      const preparation = ensurePaperPreparation(submissionOwner, item.id, id, draftStore);
      await preparation.begin({ conversationId: id, requestId, paperId: item.id, paperContextSnapshotId: snapshot.snapshotId, expectedContextHash: snapshot.contextHash, text, userInput, ...(frozenModel === undefined ? {} : { model: frozenModel }), ...(retrievalTraceId === undefined ? {} : { retrieval: { traceId: retrievalTraceId, query: text } }) }, {
        sourceCount: manifest.items.length, imageCount: manifest.items.filter(source => source.imageContentSha256 !== null).length,
        queuedInputId: queued ?? null, draftAcknowledgment, ...(originalQueueInput && originalQueueRevision !== undefined ? {queueInput: originalQueueInput, queueRevision: originalQueueRevision} : {}),
      });
      const state = preparation.getSnapshot();
      if (state.phase !== "ready" || !state.result) throw state.cause ?? new Error("论文准备结果尚待核对");
      const preview = state.result;
      if (paperEpochRef.current !== epoch || identityRef.current.paperId !== item.id) return;
      const pending = { preview, text, sourceCount: manifest.items.length, imageCount: manifest.items.filter((source) => source.imageContentSha256 !== null).length, ...(retrievalTraceId === undefined ? {} : { retrievalTraceId }), ...(queued === undefined ? {} : { queuedInputId: queued }) } satisfies PendingPaperSubmission;
      setPaperOwnerPendingSubmission(submissionOwner,pending);
      if (generation === submissionGenerationRef.current) notify.info("已准备论文上下文，请确认后发送");
      return;
    } catch (reason: unknown) {
      if (paperEpochRef.current !== epoch || submissionGenerationRef.current !== generationForUi) return;
      if (!submissionOwner?.preparation?.unresolved()) draftResources.restore(text,userInput);
      const message = `${stage}：${aiAssistanceErrorMessage(reason)}`; setError(message); notify.error(message);
    } finally { if (submissionOwner) endPaperOwnerWork(submissionOwner.key,submissionOwner); if (paperEpochRef.current === epoch && submissionGenerationRef.current === generationForUi) setBusy(false); }
  }

  async function enqueue(text: string, originalDraft = text, commandInput?: readonly AgentUserInput[]): Promise<void> {
    if (!text || workBusy || selectedConversation?.archived || paperSteering.unresolved || speechInputState !== "idle"
      || pendingSubmission !== undefined || pendingRetrieval !== undefined || (activeTurn === undefined && !inputResources.commandPending && !draftResources.draft().queuePlacement)) return;
    const userInput = commandInput ?? draftResources.input(text);
    if (queueState === undefined) {
      setMessage(text);
      const message = "待发送队列仍在加载，暂时无法排队；请稍后再按 Tab。";
      setError(message);
      notify.warning(message);
      return;
    }
    const current = queueController.current?.getSnapshot().queue;
    if (!current || queueControllerState?.editing || queueControllerState?.uncertain || (draftResources.draft().queuePlacement && current.preparingInputId)) return;
    setBusy(true); setError(undefined);
    try {
      const applied = await queueController.current?.edit(queueDraftEdit(draftResources.draft(), current, text, userInput), captureDraftAcknowledgment(originalDraft));
      if (!applied) throw new Error("队列操作未应用，请刷新后重试。");
      autoPrepare.markQueued();
      notify.success("回答完成后会继续发送这条消息");
    } catch (reason: unknown) {
      const message = aiAssistanceErrorMessage(reason); setError(message); notify.error(message);
    } finally { setBusy(false); }
  }

  return { send, enqueue };
}

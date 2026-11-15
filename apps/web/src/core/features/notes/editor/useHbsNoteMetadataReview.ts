import { useEffect, useMemo, useRef, useState } from "react";
import type { HbsSession } from "@research-assistant/core/hbs";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import { textPatchGateway, subscribeTextPatches, type TextPatchPageReview, type TextPatchResolution } from "../../agent/textPatchGateway";
import { clearNoteRecoveryDraft, loadNoteRecoveryDraft, saveNoteRecoveryDraft } from "../workspace/noteRecovery";
import { getAppStorage } from "../../../runtime/appStorage";
import { flushStorage } from "../../../runtime/storagePersistence";
import { getAppRuntime } from "../../../runtime/appRuntime";
import { noteReviewErrorMessage } from "./noteReviewRecovery";
import { acknowledgeNoteReviewRequest, notifyNoteReviewSaveState, registerNoteReviewStateContributor, type NoteReviewSaveState } from "./noteTextEditorPorts";

interface MetadataOwner { readonly conversationId: string; readonly proposalId: string; readonly generation: number; readonly page: TextPatchPageReview }
/** Metadata still belongs to its existing native proposal transaction. Reading
 * it here never creates another body writer or recomputes HBS review units. */
export function useHbsNoteMetadataReview(session: HbsSession<NoteDocumentV1>, enabled: boolean) {
  const [owner, setOwner] = useState<MetadataOwner>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [request, setRequest] = useState<TextPatchResolution>();
  const pending = useRef<TextPatchResolution | undefined>(undefined);
  const dispatched = useRef(false);
  const lastAction = useRef<"keep" | "undo">("keep");
  const mounted = useRef(true);
  const inspectionFailed=useRef(false);
  const pageId = session.getSnapshot().content.pageId;
  const read = async (): Promise<MetadataOwner | undefined> => {
    if (getAppRuntime().mode === "browser") return undefined;
    for (const conversationId of await textPatchGateway.forPage(pageId)) {
      const review = await textPatchGateway.review(conversationId,undefined,undefined,pageId);
      const page = review?.pages.find(item => item.pageId === pageId && (item.structuralMetadataPending ?? item.metadataPending));
      if (review?.state === "pending" && page) return { conversationId, proposalId: review.proposalId, generation: review.generation, page };
    }
    return undefined;
  };
  useEffect(() => {
    mounted.current = true;
    if (!enabled) return;
    let generation = 0;
    const refresh = () => { const sequence = ++generation;
      void read().then(value => { if (mounted.current && sequence === generation) {setOwner(value);if(inspectionFailed.current){inspectionFailed.current=false;setError(undefined);}} })
        .catch(reason => { if (mounted.current && sequence === generation) {inspectionFailed.current=true;setError(noteReviewErrorMessage(reason));} });
    };
    refresh(); const stop = subscribeTextPatches(refresh);
    return () => { mounted.current = false; generation++; stop(); };
  }, [enabled, pageId]);
  const execute = async (action: "keep" | "undo"): Promise<void> => {
    if (!enabled || busy || !owner && !pending.current) return;
    if (!pending.current) lastAction.current = action;
    setBusy(true);
    try {
      const state = session.getSnapshot();
      if (!pending.current) {
        if (state.pending) throw new Error("MODEL_METADATA_SAVE_REQUIRED：正文原保存请求尚未确认，请先重试或核对该请求。");
        // The legacy no-chunk operation is a whole-page decision. Never let a
        // metadata button silently consume the new granular body decisions.
        if (state.changes.length || state.conflicts.length) throw new Error("MODEL_METADATA_BODY_PENDING：请先完成正文的 Keep / Undo，再确认页面及关联变化。");
        await session.save();
        const saved = session.getSnapshot();
        if (saved.pending || saved.preservationError || saved.conflicts.length || !session.adapter.equal(saved.content, saved.observed.saved)
          || !session.adapter.equal(saved.baselineProjection, saved.observed.baseline)) throw new Error("MODEL_METADATA_SAVE_REQUIRED：正文保存尚未确认，请先核对保存结果。");
        const current = await read();
        if (!current) { if (mounted.current) setOwner(undefined); return; }
        if (current.page.chunks?.length) throw new Error("MODEL_METADATA_BODY_PENDING：原提案仍有正文变更，请先完成正文审阅。");
        pending.current = { conversationId: current.conversationId, proposalId: current.proposalId, generation: current.generation,
          requestId: crypto.randomUUID(), pageId, action };
        dispatched.current = false;
        setRequest(pending.current);
      }
      const request = pending.current;
      const latest = session.getSnapshot();
      saveNoteRecoveryDraft(pageId, latest.observed.version, latest.content, { document: latest.content, baseline: latest.baselineProjection, request });
      await flushStorage(getAppStorage());
      dispatched.current = true;
      await textPatchGateway.resolve(request);
      acknowledgeNoteReviewRequest(pageId,request.requestId);
      pending.current = undefined;
      dispatched.current = false;
      setRequest(undefined);
      await session.syncLatest(); await session.flushPreservation();
      if (loadNoteRecoveryDraft(pageId)?.review?.request?.requestId === request.requestId) clearNoteRecoveryDraft(pageId);
      if (mounted.current) { setOwner(await read()); setError(undefined); }
    } catch (reason) { if (mounted.current) setError(noteReviewErrorMessage(reason)); throw reason; }
    finally { if (mounted.current) setBusy(false); }
  };
  const wasDispatched = dispatched.current;
  const saveState = useMemo<NoteReviewSaveState>(() => ({ status: busy ? "saving" : error ? "failed" : "saved", pendingChanges: owner ? 1 : 0,
    ...(busy ? {message:`MODEL_METADATA_SAVING：正在核对页面及关联修改${request ? `（请求 ${request.requestId}）` : ""}。`}
      : error ? {message:request ? `${wasDispatched ? "MODEL_METADATA_RESULT_UNKNOWN：原页面及关联请求" : "MODEL_METADATA_PRESERVATION_FAILED：页面及关联请求"} ${request.requestId} ${wasDispatched ? "尚未核对" : "尚未发送"}；${error}` : `MODEL_METADATA_REVIEW_FAILED：${error}`} : {}) }), [busy,error,owner,request,wasDispatched]);
  const live = useRef({saveState,execute}); live.current = {saveState,execute};
  const contributes = enabled && (!!owner || !!request);
  useEffect(() => {
    if (!contributes) return;
    return registerNoteReviewStateContributor(pageId, {source:"metadata",getState:()=>live.current.saveState,
      retry:()=>live.current.execute(pending.current?.action === "undo" ? "undo" : pending.current?.action === "keep" ? "keep" : lastAction.current),
      recovery:()=>{const request=pending.current;if(!request)return undefined;const state=session.getSnapshot();
        return{document:state.content,baseline:state.baselineProjection,request};},
    });
  },[contributes,pageId,session]);
  useEffect(()=>{if(contributes)notifyNoteReviewSaveState();},[contributes,saveState]);
  const resolve = (action: "keep" | "undo") => { void execute(action).catch(()=>{}); };
  return { metadataPending: !!owner, metadataKind: owner?.page.kind, error, busy, request,
    resolve, restore: () => resolve(pending.current?.action === "undo" ? "undo" : pending.current?.action === "keep" ? "keep" : lastAction.current) };
}

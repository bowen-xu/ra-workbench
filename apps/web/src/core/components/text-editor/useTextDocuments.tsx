import { Dialog } from "../overlay/OverlayDialog";
import { DialogHeading } from "@ariakit/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { getAppStorage } from "../../runtime/appStorage";
import { flushStorage } from "../../runtime/storagePersistence";
import { notify } from "../../services/notifications";
import { getReaderCloseCoordinator } from "../../workspace/DocumentCloseCoordinator";
import { usePublishWorkspaceTabs } from "../workspace-tabs/WorkspaceTabs";
import { TextEditor } from "./TextEditor";
import { OPEN_TEXT_DOCUMENT, TEXT_DOCUMENT_REVEALED, type TextDocumentRequest } from "./textDocumentEvents";
import type { WorkspaceView } from "../../useWorkspaceNavigation";
import { TextRecoveryCheckpoints } from "./TextRecoveryCheckpoints";
import {loadTextTabs,restoreTextTab} from "./textTabSession";
import {writeWorkspaceState} from "../../runtime/workspaceState";

interface DocumentSession extends TextDocumentRequest {
  readonly content: string;
  readonly baseline: string;
  readonly saved: string;
  readonly saving: boolean;
  readonly savedBaseline: string;
}
const recoveryKey = (id: string) => `research-assistant:text-draft:${encodeURIComponent(id)}`;
function checkpointError(id: string, reason: unknown, saved = false): string {
  const cause = reason instanceof Error ? `${reason.name}: ${reason.message}` : typeof reason === "string" ? reason : "没有可用的原始错误信息";
  return `${saved ? "正文已保存，但更新文本恢复草稿失败" : "文本草稿暂存失败"}（text_recovery_checkpoint_failed；文档 ${id}）：${cause}。请保存或导出当前内容后重试。`;
}
function restore(request: TextDocumentRequest): DocumentSession {
  const initial = { ...request, baseline: request.baseline ?? request.content, saved: request.content, savedBaseline: request.baseline ?? request.content, saving: false };
  if (request.readOnly) return initial;
  try {
    const raw = getAppStorage().getItem(recoveryKey(request.id));
    const draft: unknown = raw ? JSON.parse(raw) : null;
    if (draft && typeof draft === "object" && "content" in draft && "baseline" in draft && "saved" in draft
      && typeof draft.content === "string" && typeof draft.baseline === "string" && typeof draft.saved === "string") {
      if (draft.saved === request.content) return { ...initial, content: draft.content, baseline: draft.baseline };
      if (draft.content === draft.saved && "savedBaseline" in draft && draft.baseline === draft.savedBaseline) return initial;
      notify.info("来源内容已变化，已用当前来源对比恢复草稿；请审阅 Diff 后保存。");
      return { ...initial, content: draft.content, baseline: request.content };
    }
  } catch { notify.error("无法读取文本恢复草稿。"); }
  return initial;
}
function exportText(document: DocumentSession, content: string): void {
  const url = URL.createObjectURL(new Blob([content], { type: "text/plain;charset=utf-8" }));
  const anchor = window.document.createElement("a");
  anchor.href = url; anchor.download = document.path.split(/[\\/]/).at(-1) ?? "document.txt";
  anchor.click(); window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export function useTextDocuments(owner: WorkspaceView = "settings", navigate?: (owner: WorkspaceView, ready: () => void) => boolean) {
  const [documents, setDocuments] = useState<readonly DocumentSession[]>([]);
  const [restoredTabs] = useState(loadTextTabs);
  const [restoring,setRestoring] = useState(restoredTabs.length>0);
  const [recovery] = useState(() => new TextRecoveryCheckpoints(getAppStorage));
  const checkpoint = useCallback((document: DocumentSession): void => {
    if (!document.readOnly) recovery.write(recoveryKey(document.id), document);
  }, [recovery]);
  const [closingId, setClosingId] = useState<string>();
  const closeDecision = useRef<((allow: boolean) => void) | undefined>(undefined);
  useEffect(() => () => { closeDecision.current?.(false); }, []);
  const [activeId, setActiveId] = useState<string>();
  const live = useRef(documents); live.current = documents;
  function updateDocuments(update: (documents: readonly DocumentSession[]) => readonly DocumentSession[]): void {
    live.current = update(live.current);
    setDocuments(live.current);
  }
  const active = useRef(activeId); active.current = activeId;
  const ownerRef = useRef(owner); ownerRef.current = owner;
  const navigator = useRef(navigate); navigator.current = navigate;
  const lastByOwner = useRef(new Map<WorkspaceView, string>());
  const restorationAllowed = useRef(true);
  useEffect(() => {
    if (!restoredTabs.length) return;
    let current = true;
    void Promise.allSettled(restoredTabs.map(restoreTextTab)).then(results=>{
      if (!current) return;
      const reopened=results.flatMap((result,index)=>{
        if (result.status === "fulfilled") return [restore(result.value)];
        notify.error(`workspace_text_restore_failed：无法恢复文本标签（${restoredTabs[index]!.id}）：${result.reason instanceof Error ? result.reason.message : String(result.reason)}`);
        return [];
      });
      updateDocuments(existing=>[...reopened.filter(tab=>!existing.some(value=>value.id===tab.id)),...existing]);
      const selected=restoredTabs.find(tab=>tab.active && tab.owner===ownerRef.current && reopened.some(value=>value.id===tab.id));
      if (selected && !active.current && restorationAllowed.current) { setActiveId(selected.id);lastByOwner.current.set(selected.owner,selected.id); }
      setRestoring(false);
    });
    return ()=>{current=false;};
  }, [restoredTabs]);
  useEffect(() => {
    if (!restoring) writeWorkspaceState("text-tabs",documents.flatMap(document=>document.reopen ? [{id:document.id,owner:document.owner ?? "settings",reopen:document.reopen,
      active:document.id===activeId,...(document.title ? {title:document.title} : {})}] : []));
  }, [documents,activeId,restoring]);
  const deactivate = useCallback(() => {restorationAllowed.current=false;setActiveId(undefined);}, []);
  const forgetOwner = useCallback((owner: WorkspaceView) => { lastByOwner.current.delete(owner); }, []);
  function activate(id: string): void {
    const document = live.current.find(candidate => candidate.id === id);
    if (!document) return;
    lastByOwner.current.set(document.owner ?? "settings", id);
    setActiveId(id);
  }
  function restoreOwner(owner: WorkspaceView): () => void {
    const id = lastByOwner.current.get(owner);
    return () => { if (id) activate(id); };
  }
  useEffect(() => {
    const open = (event: Event) => {
      const request = (event as CustomEvent<TextDocumentRequest>).detail;
      if (!request?.id || typeof request.content !== "string") return;
      const existing = live.current.find(document => document.id === request.id);
      const owner = existing?.owner ?? request.owner ?? ownerRef.current;
      const ready = () => {
        const current = live.current.find(document => document.id === request.id);
        if (!current || !current.saving && current.content === current.saved && current.baseline === current.savedBaseline) {
          const document = restore({ ...request, owner });
          updateDocuments(documents => current ? documents.map(candidate => candidate.id === request.id ? document : candidate) : [...documents, document]);
        }
        lastByOwner.current.set(owner, request.id);
        setActiveId(request.id);
        window.dispatchEvent(new Event(TEXT_DOCUMENT_REVEALED));
      };
      if (navigator.current ? navigator.current(owner, ready) : (ready(), true)) event.preventDefault();
    };
    window.addEventListener(OPEN_TEXT_DOCUMENT, open);
    return () => window.removeEventListener(OPEN_TEXT_DOCUMENT, open);
  }, []);
  useEffect(() => {
    recovery.retain(documents.map(document => recoveryKey(document.id)));
    const unsubs = documents.map(document => getReaderCloseCoordinator().register(`text:${document.id}`, {
      isActive: () => active.current === document.id,
      prepare: async () => {
        const current = live.current.find(candidate => candidate.id === document.id);
        if (!current || current.saving) return !current;
        try { checkpoint(current); await flushStorage(getAppStorage()); return true; }
        catch (reason) { notify.error(checkpointError(current.id, reason)); return false; }
      },
      prepareClose: async () => {
        const current = live.current.find(candidate => candidate.id === document.id);
        if (!current) return true;
        if (current.saving) return false;
        if (current.content === current.saved && current.baseline === current.savedBaseline) return true;
        if (closeDecision.current) return false;
        return new Promise<boolean>(resolve => { closeDecision.current = resolve; setClosingId(document.id); });
      },
      commit: () => {
        if (lastByOwner.current.get(document.owner ?? "settings") === document.id) lastByOwner.current.delete(document.owner ?? "settings");
        setDocuments(current => current.filter(candidate => candidate.id !== document.id));
        setActiveId(current => current === document.id ? undefined : current);
      },
    }));
    return () => unsubs.forEach(unsub => unsub());
  }, [documents, checkpoint, recovery]);
  function change(id: string, content: string, baseline: string): void {
    const current = live.current.find(document => document.id === id);
    if (!current || current.readOnly) return;
    const next = { ...current, content, baseline };
    try { checkpoint(next); } catch (reason) { notify.error(checkpointError(next.id, reason)); }
    updateDocuments(previous => previous.map(document => document.id === id ? next : document));
  }
  async function save(id: string): Promise<boolean> {
    const current = live.current.find(document => document.id === id);
    if (!current || current.saving || current.readOnly) return false;
    updateDocuments(previous => previous.map(document => document.id === id ? { ...document, saving: true } : document));
    try {
      if (current.save) await current.save(current.content, current.baseline);
      else exportText(current, current.content);
      const latest = live.current.find(document => document.id === id);
      if (latest) {
        try { checkpoint({ ...latest, saved: current.content, savedBaseline: current.baseline, saving: false }); await flushStorage(getAppStorage()); }
        catch (reason) { notify.error(checkpointError(id, reason, true)); }
      }
      updateDocuments(previous => previous.map(document => document.id === id ? { ...document, saved: current.content, savedBaseline: current.baseline, saving: false } : document));
      return true;
    } catch (reason) {
      notify.error(`保存文本失败：${String(reason)}`);
      updateDocuments(previous => previous.map(document => document.id === id ? { ...document, saving: false } : document));
      return false;
    }
  }
  const closingBusy = documents.some(document => document.id === closingId && document.saving);
  usePublishWorkspaceTabs("text", documents.map(document => ({
    id: `text:${document.id}`, owner: document.owner ?? "settings", title: document.title ?? document.path.split(/[\\/]/).at(-1) ?? document.path,
    active: activeId === document.id, dirty: document.content !== document.saved || document.baseline !== document.savedBaseline, pending: document.content !== document.baseline,
    select: () => activate(document.id), close: () => { void getReaderCloseCoordinator().close(`text:${document.id}`); },
  })));
  return { activeId, deactivate, forgetOwner, restoreOwner, surface: <> {closingId && <Dialog open portal className="text-save-dialog" aria-label="保存文本修改" hideOnInteractOutside={false} hideOnEscape={() => { if (closingBusy) return false; closeDecision.current?.(false); closeDecision.current = undefined; setClosingId(undefined); return false; }}>
    <DialogHeading render={<h2 />}>保存修改？</DialogHeading>
    <p>关闭前是否保存对“{documents.find(document => document.id === closingId)?.title ?? documents.find(document => document.id === closingId)?.path}”的修改？</p>
    <div><button aria-label="取消关闭文本" disabled={closingBusy} onClick={() => { closeDecision.current?.(false); closeDecision.current = undefined; setClosingId(undefined); }}>取消</button>
    <button aria-label="不保存并关闭文本" disabled={closingBusy} onClick={() => { void (async () => {
      getAppStorage().removeItem(recoveryKey(closingId)); await flushStorage(getAppStorage());
      closeDecision.current?.(true); closeDecision.current = undefined; setClosingId(undefined);
    })().catch(reason => notify.error(`无法清除恢复草稿，请重试：${reason instanceof Error ? reason.message : String(reason)}`)); }}>不保存</button>
    <button disabled={closingBusy} onClick={() => { void save(closingId).then(ok => { if (ok) { const latest = live.current.find(document => document.id === closingId); const clean = latest?.content === latest?.saved && latest?.baseline === latest?.savedBaseline; closeDecision.current?.(clean); closeDecision.current = undefined; setClosingId(undefined); } }); }}>保存</button></div>
  </Dialog>}<div className="text-documents-host" hidden={activeId === undefined}>
    {documents.map(document => <div className="text-document-host" hidden={document.id !== activeId} key={document.id}>
      <TextEditor documentId={document.id} path={document.path} value={document.content} baseline={document.baseline}
        readOnly={document.readOnly === true} dirty={document.content !== document.saved || document.baseline !== document.savedBaseline} saving={document.saving}
        onChange={(value, baseline) => change(document.id, value, baseline)} onSave={() => { void save(document.id); }} />
    </div>)}
  </div></> };
}

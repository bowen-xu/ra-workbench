import { useNotificationState } from "../../../services/useNotificationState";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Settings2, X } from "lucide-react";
import { agentContextKey, isAgentContextGroup, selectAgentContext, type AgentContextSelection, type AgentContextState } from "@research-assistant/core/agent";
import { AgentContextRecommendations } from "./AgentContextRecommendations";
import type { useContextRecommendations } from "./useContextRecommendations";
import type { AgentContextGateway } from "../../../services/agentContextGateway";
import { contextCatalogErrorMessage } from "../../../services/agentContextCatalogError";
import { ContextCatalogPages } from "./ContextCatalogPages";
import { selectedCatalogItem, withoutCatalogMembers } from "./contextCatalogSelection";
import type { AgentContextDirectory } from "./contextDirectoryGateway";
import { contextDirectoryGateway } from "./contextDirectoryGateway";
import { getAppStorage } from "../../../runtime/appStorage";
import { ControlHint } from "../../../components/ControlHint";
import "./AgentContextControl.css";

/** Persistent context lives in the header; turn references belong to composer @ bindings. */
export function AgentContextControl({recommendations, onChange, gateway, disabled, conversationId, onDirectoryChanged, captureDirectoryNavigation, beforeDirectoryChange, pending = false}: {
  readonly state: AgentContextState;
  readonly recommendations: ReturnType<typeof useContextRecommendations>;
  readonly onChange: (state: AgentContextState) => void | Promise<void>;
  readonly gateway: AgentContextGateway;
  readonly disabled: boolean;
  readonly pending?: boolean;
  readonly beforeDirectoryChange?: () => Promise<void>;
  readonly conversationId: string;
  readonly onDirectoryChanged?: (directory: AgentContextDirectory | null) => void;
  readonly captureDirectoryNavigation?: () => () => boolean;
}): React.JSX.Element {
  const [open, setOpen] = useState(false), [query, setQuery] = useState("");
  const [results, setResults] = useState<readonly AgentContextSelection[]>([]);
  const [loading, setLoading] = useState(false), [editing, setEditing] = useState(false), [, setError] = useNotificationState("error");
  const dialog = useRef<HTMLDialogElement>(null), trigger = useRef<HTMLButtonElement>(null), wasOpen = useRef(false);
  const mutation = useRef(false);
  const closeRequested = useRef(false);
  const [directory, setDirectory] = useState<AgentContextDirectory | null>(null);
  const [directoryItem, setDirectoryItem] = useState<AgentContextSelection>();
  const [optimistic, setOptimistic] = useState<AgentContextState | null>(null);
  const autoKey = `research-assistant:context-auto:${conversationId}`;
  const recommendation = recommendations.items[0];
  const base = optimistic ?? recommendations.effective;
  const legacyProject = directory && !base.persistent.some(item => item.resource.kind === "manuscript" && item.resource.resourceId === directory.manuscriptId)
    ? results.find(item => item.resource.kind === "manuscript" && item.resource.resourceId === directory.manuscriptId) ?? (directoryItem?.resource.resourceId === directory.manuscriptId ? directoryItem : undefined) : undefined;
  const displayed = legacyProject && legacyProject.resource.kind === "manuscript" ? selectAgentContext(withoutCatalogMembers(base, legacyProject), {...legacyProject,resource:{...legacyProject.resource,newMemberAccess:"readWrite",parts:legacyProject.resource.parts.map(part => ({...part,access:"readWrite"}))}}, "persistent") : base;
  const locked = disabled || editing;
  const directories = gateway.directories ?? contextDirectoryGateway;
  function finishClose(): void {
    // Let the native modal restore its old focus before React returns focus to
    // the current trigger; removing an open dialog can restore it too late.
    dialog.current?.close();
    setOpen(false);
  }
  function close(): void {if(mutation.current)closeRequested.current=true;else finishClose();}
  useEffect(() => {
    if (open && !dialog.current?.open) dialog.current?.showModal();
    if (open) wasOpen.current = true;
    else if (wasOpen.current && !locked) {
      const frame = requestAnimationFrame(() => {
        trigger.current?.focus();
        wasOpen.current = false;
      });
      return () => cancelAnimationFrame(frame);
    }
  }, [open, locked]);
  useEffect(() => {
    if (!open) return;
    let active = true;
    void (pending ? Promise.resolve(null) : directories.read(conversationId)).then(async value => {
      if(!active)return;setDirectory(value);
      if(value && gateway.catalog) {
        const rows = await gateway.catalog(value.title);
        if(active)setDirectoryItem(rows.find(item => item.resource.kind === "manuscript" && item.resource.resourceId === value.manuscriptId));
      }
    }).catch(() => {if(active)setError("无法核对写作项目权限，请重试。");});
    return () => {active=false;};
  }, [open, pending, conversationId, gateway]);
  useEffect(() => {
    if (!open) return;
    const escape = (event: KeyboardEvent): void => {
      if (event.key !== "Escape" || event.isComposing) return;
      event.preventDefault(); event.stopPropagation(); close();
    };
    document.addEventListener("keydown", escape, true);
    return () => document.removeEventListener("keydown", escape, true);
  }, [open]);
  useEffect(() => {
    if (!open) return;
    let active = true;
    setLoading(true); setError(undefined);
    const timer = setTimeout(() => {
      void (gateway.catalog ?? gateway.search)(query).then(rows => { if (active) setResults(rows); })
        .catch(reason => { if (active) setError(contextCatalogErrorMessage(reason)); })
        .finally(() => { if (active) setLoading(false); });
    }, 150);
    return () => { active = false; clearTimeout(timer); };
  }, [open, query, gateway]);
  async function change(item: AgentContextSelection, remove = false): Promise<void> {
    if (locked || mutation.current) return;
    mutation.current = true; setEditing(true);
    let completed=false;
    try {
      const next = withoutCatalogMembers(displayed, item);
      const changed = remove ? next : selectAgentContext(next, item, "persistent");
      const editable = changed.persistent.filter(row => row.resource.kind === "manuscript" && row.resource.groupAccess !== "readOnly" && (row.resource.newMemberAccess === "readWrite" || row.resource.parts.some(part => part.selected && part.access === "readWrite")));
      if (editable.length > 1) throw new Error("一个对话同时只能编辑一个写作项目；其它项目可设为只读。");
      setOptimistic(changed);
      const target = editable[0]?.resource.resourceId ?? null, previous = directory?.manuscriptId ?? null;
      // The save may outlive this conversation's selection. Capture ownership
      // before awaiting; a stale completion saves data but must not navigate.
      const canNavigate = target !== previous ? captureDirectoryNavigation?.() : undefined;
      let updated = directory;
      if (target !== previous) {
        await beforeDirectoryChange?.();
        updated = await directories.set(conversationId, previous, target);
      }
      try {await onChange(changed);} catch(reason) {
        if(target !== previous) await directories.set(conversationId, target, previous);
        throw reason;
      }
      setDirectory(updated);
      if(target !== previous) {finishClose();if (!canNavigate || canNavigate()) onDirectoryChanged?.(updated);}
      recommendations.clear(item);
      if(isAgentContextGroup(item.resource)) for(const part of item.resource.parts) recommendations.clear({resource:part.resource,label:part.label});
      getAppStorage().setItem(autoKey,"manual");
      setError(undefined);
      completed=true;
    } catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { mutation.current = false; setEditing(false);setOptimistic(null);if(closeRequested.current&&completed)finishClose();closeRequested.current=false; }
  }
  const availableKeys = new Set(results.map(item => agentContextKey(item.resource)));
  const rows = [...displayed.persistent.filter(item => !availableKeys.has(agentContextKey(item.resource)) && !results.some(row => selectedCatalogItem(row, { ...displayed, persistent: [item] }))), ...results];
  return <>
    <div className="agent-context-header-controls">
      {recommendation && <AgentContextRecommendations primary={recommendation} selected={recommendations.selected} suggestions={recommendations.items} disabled={locked} onChange={recommendations.choose} {...(recommendations.error ? {error: recommendations.error} : {})}/>}
      <ControlHint content="上下文管理"><button ref={trigger} type="button" className="agent-context-settings" aria-label="上下文管理" aria-haspopup="dialog" disabled={editing} onClick={() => setOpen(true)}><Settings2 aria-hidden="true" size={17} /></button></ControlHint>
    </div>
    {open && createPortal(<dialog ref={dialog} className="agent-context-dialog" aria-label="上下文管理" onCancel={event => { event.preventDefault(); close(); }}>
      <header><strong>上下文管理</strong><button type="button" aria-label="关闭上下文管理" onClick={close}><X aria-hidden="true" size={17} /></button></header>
      <section className="agent-context-control">
        <div className="agent-context-picker">
          <input aria-label="搜索上下文" placeholder="搜索文献、笔记、写作项目" value={query} onChange={event => setQuery(event.target.value)} />
          {loading ? <div className="context-catalog-pager"><div/><div className="context-catalog-viewport"><p role="status">读取材料……</p></div><div/></div>
            : <ContextCatalogPages key={query} rows={rows} state={displayed} disabled={locked} onChange={(item, remove) => void change(item, remove)}/>}
        </div>

      </section>
    </dialog>, document.body)}
  </>;
}

import { ControlHint } from "../ControlHint";
import { createContext, useContext, useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { FileCode2, FileText, House, X } from "lucide-react";
import type { WorkspaceView } from "../../contracts/workbench";
import { getReaderCloseCoordinator } from "../../workspace/DocumentCloseCoordinator";
import "./WorkspaceTabs.css";
import { WorkspaceTabsRegistry } from "./WorkspaceTabsRegistry";
import { usePointerReorder } from "../reorder/usePointerReorder";
import { ReorderGroup, ReorderItem } from "../reorder/ReorderGroup";
import {readWorkspaceState, writeWorkspaceState, workspaceStrings} from "../../runtime/workspaceState";

import type { WorkspaceTab, WorkspacePresentation } from "../../contracts/workbench";
export type { WorkspaceTab } from "../../contracts/workbench";

const Context = createContext<WorkspaceTabsRegistry | null>(null);
export function WorkspaceTabsProvider({ children }: { readonly children: ReactNode }): React.JSX.Element {
  const [registry] = useState(() => new WorkspaceTabsRegistry(readWorkspaceState("tab-order",[],workspaceStrings),order=>writeWorkspaceState("tab-order",order)));
  return <Context.Provider value={registry}>{children}</Context.Provider>;
}

/** Standalone workspaces retain their local tab UI when no shell is present. */
export function usePublishWorkspaceTabs(group: string, tabs: readonly WorkspaceTab[], enabled = true): boolean {
  const registry = useContext(Context);
  const latest = useRef(tabs); latest.current = tabs;
  useEffect(() => {
    if (!enabled) return;
    registry?.set(group, latest.current);
  });
  useEffect(() => {
    if (!enabled) return;
    return () => { registry?.remove(group); };
  }, [registry, group, enabled]);
  return registry !== null && enabled;
}

export function WorkspaceTabBar({ owner, homeOwner = owner, textActive, navigate }: {
  readonly owner: WorkspaceView;
  readonly homeOwner?: WorkspaceView;
  readonly textActive: boolean;
  readonly navigate: (owner: WorkspaceView, destination: string | undefined, ready: () => void, presentation?: WorkspacePresentation) => boolean;
}): React.JSX.Element {
  const registry = useContext(Context)!;
  const allTabs = useSyncExternalStore(registry.subscribe, registry.getSnapshot);
  const home=allTabs.find(tab=>tab.home && tab.owner===homeOwner) ?? allTabs.find(tab=>tab.home && tab.owner==="library");
  const fileTabs=allTabs.filter(tab=>!tab.home);
  const host=useRef<HTMLDivElement>(null);
  const drag=usePointerReorder({ids:fileTabs.map(tab=>tab.id),axis:"horizontal",scrollContainer:host,
    onReorder:({source,target,after})=>registry.move(source,target,after)});
  const tabs=[...(home ? [home] : []),...drag.order.flatMap(id=>fileTabs.filter(tab=>tab.id===id))];
  const textGroups = tabs.filter(tab => tab.id.startsWith("text:"));
  const selected = tabs.find(tab => tab.active && (textActive ? textGroups.includes(tab) : !textGroups.includes(tab) && tab.owner === owner));
  const selectedRef = useRef(selected); selectedRef.current = selected;
  useEffect(() => getReaderCloseCoordinator().register("workspace:active-tab", {
    isActive: () => Boolean(selectedRef.current?.close),
    // Each workspace callback retains its existing save/conflict close boundary.
    prepare: async () => true,
    commit: () => selectedRef.current?.close?.(),
  }), []);
  const refs = useRef(new Map<string, HTMLButtonElement>());
  useEffect(() => {
    const root=host.current, button=refs.current.get(selected?.id ?? "");
    if(!root || !button || selected?.home)return;
    const rail=root.getBoundingClientRect(),tab=button.parentElement!.getBoundingClientRect();
    const fixed=root.querySelector<HTMLElement>('[data-home="true"]')?.getBoundingClientRect().width ?? 0;
    const left=rail.left+fixed+6;
    // Only the horizontal strip may scroll; avoid scrolling window/toolbar ancestors.
    if(tab.left<left)root.scrollLeft-=left-tab.left;
    else if(tab.right>rail.right)root.scrollLeft+=tab.right-rail.right;
  }, [selected?.id]);
  return <ReorderGroup className="workspace-tabs" containerRef={host} axis="horizontal" order={drag.order} dragging={!!drag.drag} role="tablist" aria-label="打开的标签页" onKeyDown={event => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    const index = tabs.findIndex(tab => refs.current.get(tab.id) === document.activeElement);
    if (index < 0) return;
    event.preventDefault();
    if(event.altKey && (event.key==="ArrowLeft" || event.key==="ArrowRight")){
      const current=tabs[index],target=tabs[index+(event.key==="ArrowRight" ? 1 : -1)];
      if(current && !current.home && target && !target.home)registry.move(current.id,target.id,event.key==="ArrowRight");
      return;
    }
    const next = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : (index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
    const tab = tabs[next];
    if (tab) navigate(tab.owner, tab.destination, () => { tab.select(); refs.current.get(tab.id)?.focus(); }, tab.presentation);
  }}>
    {tabs.map(tab => <ReorderItem className="workspace-tab" itemId={tab.id} fixed={!!tab.home} axis="horizontal" offset={drag.offsetFor(tab.id)} dragging={drag.drag?.id===tab.id}
      ref={element=>{if(tab.home)return;if(element)drag.rows.current.set(tab.id,element);else drag.rows.current.delete(tab.id);}}
      data-tab-id={tab.id} data-home={tab.home || undefined} data-owner={tab.owner} data-active={tab.id === selected?.id} data-preview={tab.preview || undefined} data-pending={tab.pending || undefined} key={tab.id}>
      <ControlHint content={tab.preview ? `${tab.title}（临时标签，双击固定）` : tab.title}><button type="button" role="tab" aria-selected={tab.id === selected?.id} tabIndex={tab.id === selected?.id || !selected ? 0 : -1}
        ref={element => { if (element) refs.current.set(tab.id, element); else refs.current.delete(tab.id); }}
        aria-label={tab.home ? tab.title : undefined}
        onPointerDown={tab.home ? undefined : event=>drag.start(tab.id,event)}
        onClick={() => {if(!drag.consumeClick())navigate(tab.owner, tab.destination, tab.select, tab.presentation);}} onDoubleClick={() => tab.pin?.()}>
        {tab.home ? <House size={16} aria-hidden="true" /> : <>{tab.icon === "pdf" ? <span className="workspace-tab-pdf" aria-hidden="true">PDF</span> : tab.icon === "code" ? <FileCode2 size={15} aria-hidden="true" /> : <FileText size={15} aria-hidden="true" />}<span className="workspace-tab-title">{tab.title}</span>{tab.dirty && <span className="workspace-tab-dirty" aria-label="未保存">•</span>}{tab.pending && <span className="workspace-tab-pending" aria-label="待审变更" />}</>}
      </button></ControlHint>
      {tab.close && <button className="workspace-tab-close" type="button" aria-label={`关闭 ${tab.title}`} onClick={tab.close}><X size={13} aria-hidden="true" /></button>}
    </ReorderItem>)}
  </ReorderGroup>;
}

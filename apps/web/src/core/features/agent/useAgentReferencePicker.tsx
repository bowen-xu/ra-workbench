// Input bindings adapted from Codex TUI input_submission.rs at
// e363b08c9175ac1cbe5893615dd2cb9ddf95043b (Apache-2.0, Copyright 2025 OpenAI).
import { useState, useEffect, useRef, type RefObject } from "react";
import {ArrowLeft, Search} from "lucide-react";
import type { AgentMentionBinding, AgentReferenceInput, AgentResourceScope } from "@research-assistant/core/agent";
import { AgentResourceList, filterAgentResources, useAgentResources } from "./AgentResourceList";
import type { AgentComposerInputHandle, AgentComposerKeyEvent } from "./AgentComposerTextInput";
import type { AgentResourceGateways } from "./agentResourceGateways";
import { ActionMenuItem, InputActionMenu, useActionMenuStore } from "../../components/ActionMenu";

/** Only @ selects resources; slash commands have a separate catalog and dispatch. */
export function useAgentReferencePicker({text, input, insert, bindings, gateways, scope}: {
  readonly text: string; readonly input: RefObject<AgentComposerInputHandle | null>;
  readonly insert: (reference: AgentReferenceInput, start?: number, end?: number) => void;
  readonly bindings: () => readonly AgentMentionBinding[];
  readonly gateways?: AgentResourceGateways; readonly scope?: AgentResourceScope;
}) {
  const [trigger, setTrigger] = useState<{start: number; end: number; query: string} | null>(null);
  const [index, setIndex] = useState<number | null>(null);
  const [fileSearch, setFileSearch] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const dismissed = useRef<{value: string; caret: number} | null>(null);
  function close(value = text, caret = input.current?.selectionStart ?? value.length): void {
    dismissed.current = {value, caret}; setTrigger(null); setFileSearch(false);
  }
  const store=useActionMenuStore({open:trigger!==null,setOpen:open=>{if(!open)close();},placement:"top-start",virtualFocus:true});
  useEffect(()=>{
    const element=input.current instanceof HTMLElement?input.current:input.current?.element;
    store.setAnchorElement(element??null);store.setDisclosureElement(element??null);
  },[store,input,trigger]);
  useEffect(() => {
    if (!trigger) return;
    const dismiss = (event: PointerEvent) => {if (event.target instanceof Node && !menuRef.current?.contains(event.target) && !(input.current instanceof HTMLElement ? input.current : input.current?.element)?.contains(event.target)) close();};
    document.addEventListener("pointerdown", dismiss);
    return () => document.removeEventListener("pointerdown", dismiss);
  }, [trigger, input]);
  const catalog = useAgentResources(trigger !== null, gateways, scope, fileSearch ? trigger?.query ?? "" : undefined);
  const filtered = filterAgentResources(fileSearch ? catalog.resources.filter(resource => resource.group === "files") : catalog.resources, trigger?.query ?? "");
  const candidates = filtered.filter(candidate=>!candidate.disabled);
  const searchIndex = candidates.filter(candidate => candidate.group === "files").length;
  // Start at the first resource when there are only tools/skills, preserving @ completion.
  const selectedIndex = Math.min(index ?? (!fileSearch && searchIndex === 0 && candidates.length ? 1 : 0), Math.max(0, candidates.length - (fileSearch ? 1 : 0)));
  const resourceIndex = selectedIndex > searchIndex ? selectedIndex - 1 : selectedIndex;
  const selected = fileSearch ? candidates[selectedIndex] : selectedIndex === searchIndex ? undefined : candidates[resourceIndex];
  const files = filtered.filter(candidate => candidate.group === "files");
  const otherResources = filtered.filter(candidate => candidate.group !== "files");
  function activate(reference: AgentReferenceInput): void {
    const next = candidates.findIndex(candidate => candidate.input === reference);
    if (next >= 0) setIndex(next + (!fileSearch && next >= searchIndex ? 1 : 0));
  }
  function update(value: string, caret: number): void {
    // Resource updates may re-report the same caret. Explicit dismissal lasts
    // until the user changes the text or moves it, rather than reopening @.
    if (dismissed.current?.value === value && dismissed.current.caret === caret) return;
    dismissed.current = null;
    const match = /(?:^|\s)(@)([^\s]*)$/u.exec(value.slice(0, caret));
    const next = fileSearch && trigger && value[trigger.start] === "@" && caret > trigger.start
      ? {start: trigger.start, end: caret, query: value.slice(trigger.start + 1, caret)}
      : match ? {start: caret - match[2]!.length - 1, end: caret, query: match[2]!} : null;
    // The editor keeps @name as protocol text underneath its atomic widget.
    // Moving the caret to a selected reference must not turn it back into a query.
    const selected = next && bindings().some(binding => next.start < binding.endUtf16 && next.end > binding.startUtf16);
    // Native textarea keyup/select events can repeat an unchanged caret after
    // keyboard navigation. Keep the highlighted candidate in that case.
    if (!selected && next && trigger && next.start === trigger.start && next.end === trigger.end && next.query === trigger.query) return;
    if (fileSearch && trigger?.query && next?.query === "") {close(value, caret); return;}
    setTrigger(selected ? null : next);
    if (selected || !next || next.start !== trigger?.start) setFileSearch(false);
    setIndex(null);
  }
  function select(reference: AgentReferenceInput): void {
    insert(reference, trigger?.start ?? input.current?.selectionStart ?? text.length, trigger?.end ?? input.current?.selectionEnd ?? text.length);
    const start = trigger?.start ?? input.current?.selectionStart ?? text.length;
    const separator = start > 0 && !/\s/u.test(text[start - 1]!) ? 1 : 0;
    const caret = start + separator + reference.name.length + 2;
    close();
    requestAnimationFrame(() => { input.current?.focus(); input.current?.setSelectionRange(caret, caret); });
  }
  function keyDown(event: AgentComposerKeyEvent): boolean {
    if (!trigger || event.nativeEvent.isComposing) return false;
    if (event.key === "Escape") {event.preventDefault(); event.stopPropagation(); close(); return true;}
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault(); event.stopPropagation();
      const count = Math.max(1, candidates.length + (fileSearch ? 0 : 1));
      setIndex((selectedIndex + (event.key === "ArrowDown" ? 1 : -1) + count) % count); return true;
    }
    if (event.key === "Enter" || event.key === "Tab" && !event.shiftKey) {
      event.preventDefault(); event.stopPropagation();
      if (selected) select(selected.input); else if (!fileSearch) {setIndex(null); setFileSearch(true);}
      return true;
    }
    return false;
  }
  return {update, keyDown, select, close, menu: trigger ? <InputActionMenu store={store} className="agent-reference-picker" label="选择 Agent 资源"><div ref={menuRef}>
    {fileSearch ? <>
      <ActionMenuItem hideOnClick={false} aria-label="返回已打开的文件" className="agent-resource-item" onMouseDown={event => event.preventDefault()}
        onClick={() => {setFileSearch(false); setIndex(null); requestAnimationFrame(() => input.current?.focus());}}>
        <ArrowLeft aria-hidden="true" size={16}/><span>搜索文件</span>
      </ActionMenuItem>
      <AgentResourceList resources={filtered} status={catalog.status} groups={["files"]} onSelect={select} onActivate={activate} activeIndex={filtered.indexOf(selected!)}/>
    </> : <>
      <AgentResourceList resources={files} status={catalog.status} groups={["files"]} onSelect={select} onActivate={activate} activeIndex={files.indexOf(selected!)}/>
      <ActionMenuItem data-tab-action="forward" hideOnClick={false} aria-label="搜索" className={`agent-resource-item${selectedIndex === searchIndex ? " is-selected" : ""}`}
        onFocus={() => setIndex(searchIndex)} onPointerMove={() => setIndex(searchIndex)}
        onMouseDown={event => event.preventDefault()} onClick={() => {setIndex(null); setFileSearch(true); input.current?.focus();}} onKeyDown={event => {
          if (!event.nativeEvent.isComposing && event.key === "Tab" && !event.shiftKey) {
            event.preventDefault(); event.stopPropagation(); setIndex(null); setFileSearch(true); input.current?.focus();
          }
        }}><Search aria-hidden="true" size={16}/><span>搜索</span>{selectedIndex === searchIndex && <small className="agent-file-menu-hint">Tab 查看更多</small>}</ActionMenuItem>
      <AgentResourceList resources={otherResources} status="" groups={["tools", "skills"]} onSelect={select} onActivate={activate} activeIndex={otherResources.indexOf(selected!)}/>
    </>}
  </div></InputActionMenu> : null};
}

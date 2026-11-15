import { ControlHint } from "../ControlHint";
import { samePdfDocument, type PdfDocumentIdentity } from "@research-assistant/core/pdf-view";
import { ChevronDown, ChevronUp, ChevronLeft, ChevronRight, Minus, Plus, Search, X } from "lucide-react";
import { useEffect, useRef, useState, type RefObject } from "react";
import { ActionMenu, ActionMenuItem } from "../ActionMenu";
import { isPdfEditable } from "./PdfView";
import type { PdfViewController, PdfViewSnapshot } from "./PdfViewController";
import { Popover } from "../overlay/OverlayPopup";
import { usePopoverStore } from "@ariakit/react";

export function PdfPageControls(props: {
  pageIndex: number; pageCount: number; value: string; label?: string;
  inputRef?: RefObject<HTMLInputElement | null>; horizontal?: boolean;
  onChange: (value: string) => void; onCommit: () => void;
  onPrevious: () => void; onNext: () => void;
}): React.JSX.Element {
  const next = <ControlHint content="下一页"><button aria-label="下一页" disabled={!props.pageCount || props.pageIndex >= props.pageCount - 1} onClick={props.onNext} type="button">{props.horizontal ? <ChevronRight /> : <ChevronDown size={16} />}</button></ControlHint>;
  return <>
    <ControlHint content="上一页"><button aria-label="上一页" disabled={props.pageIndex <= 0} onClick={props.onPrevious} type="button">{props.horizontal ? <ChevronLeft /> : <ChevronUp size={16} />}</button></ControlHint>
    {!props.horizontal && next}
    <input ref={props.inputRef} aria-label={props.label ?? "PDF 页码"} inputMode="numeric" value={props.value} onChange={(event) => props.onChange(event.target.value)} onBlur={props.onCommit} onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) props.onCommit(); }} />
    <span className="pdf-page-count">/ {props.pageCount || "…"}</span>
    {props.horizontal && next}
  </>;
}

export function PdfViewControls({ controller, state, scopeRef, onShowPdf }: {
  controller: Pick<PdfViewController, "find" | "closeFind" | "focus" | "navigate" | "changeScale" | "setScale">;
  state: PdfViewSnapshot; scopeRef: RefObject<HTMLElement | null>; onShowPdf: () => void;
}): React.JSX.Element {
  const [pageInput, setPageInput] = useState(String(state.pageIndex + 1));
  const [searchSource, setSearchSource] = useState<PdfDocumentIdentity>();
  const open = samePdfDocument(searchSource, state.source);
  const setOpen = (value: boolean): void => setSearchSource(value && state.source ? { ...state.source } : undefined);
  const searchPopover = usePopoverStore({open, placement: "bottom-end"});
  const [query, setQuery] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [entireWord, setEntireWord] = useState(false);
  const [highlightAll, setHighlightAll] = useState(true);
  const searchRef = useRef<HTMLInputElement>(null);
  const searchTrigger = useRef<HTMLButtonElement>(null);
  const find = (previous = false, type: "find" | "again" = "again"): void => controller.find({ query, findPrevious: previous, type, caseSensitive, entireWord, highlightAll });
  const close = (): void => { setOpen(false); controller.closeFind(); controller.focus(); };
  useEffect(() => { setOpen(false); setQuery(""); }, [state.source?.key]);
  useEffect(() => setPageInput(String(state.pageIndex + 1)), [state.pageIndex]);
  useEffect(() => { if (open) searchRef.current?.focus(); }, [open]);
  useEffect(() => { if (open && state.ready && !state.loading) find(false, "find"); }, [query, caseSensitive, entireWord, highlightAll, open, state.source, state.loading]);
  useEffect(() => {
    const scope = scopeRef.current;
    if (!scope) return;
    const handler = handleShortcut;
    scope.addEventListener("keydown", handler);
    return () => scope.removeEventListener("keydown", handler);
  }, [scopeRef, controller, open, query, caseSensitive, entireWord, highlightAll, onShowPdf]);
  function handleShortcut(event: KeyboardEvent): void {
      if (event.isComposing || event.repeat || document.querySelector('dialog[open],[aria-modal="true"]')) return;
      if (isPdfEditable(event.target) && event.target !== searchRef.current) return;
      const key = event.key.toLowerCase();
      if ((event.metaKey || event.ctrlKey) && key === "f") { event.preventDefault(); event.stopPropagation(); onShowPdf(); setOpen(true); searchRef.current?.focus(); }
      else if ((event.metaKey || event.ctrlKey) && key === "g" && open) { event.preventDefault(); event.stopPropagation(); find(event.shiftKey); }
      else if (event.key === "Escape" && open) { event.preventDefault(); event.stopPropagation(); close(); }
  }
  function commit(): void {
    const page = Number(pageInput);
    if (Number.isInteger(page) && page >= 1 && page <= state.pageCount) { controller.navigate({ pageIndex: page - 1 }); onShowPdf(); }
    else setPageInput(String(state.pageIndex + 1));
  }
  return <div className="pdf-preview-toolbar" aria-label="PDF 阅读工具">
    <PdfPageControls pageIndex={state.pageIndex} pageCount={state.pageCount} value={pageInput} onChange={setPageInput} onCommit={commit}
      onPrevious={() => { controller.navigate({ pageIndex: state.pageIndex - 1 }); onShowPdf(); }} onNext={() => { controller.navigate({ pageIndex: state.pageIndex + 1 }); onShowPdf(); }} />
    <span className="pdf-toolbar-separator" aria-hidden="true" />
    <ControlHint content="缩小 PDF"><button type="button" aria-label="缩小 PDF" disabled={state.scalePercent <= state.scaleLimits.min * 100} onClick={() => { controller.changeScale(-.1); onShowPdf(); }}><Minus size={16} /></button></ControlHint>
    <ControlHint content="放大 PDF"><button type="button" aria-label="放大 PDF" disabled={state.scalePercent >= state.scaleLimits.max * 100} onClick={() => { controller.changeScale(.1); onShowPdf(); }}><Plus size={16} /></button></ControlHint>
    <ActionMenu className="pdf-zoom-menu" contentClassName="pdf-zoom-popup" label="PDF 缩放" trigger={<>{state.scalePercent}%<ChevronDown size={12} /></>}>
      <ActionMenuItem onClick={() => { controller.setScale({ kind: "page-width" }); onShowPdf(); }}>适应宽度</ActionMenuItem>
      {[.5, .75, 1, 1.25, 1.5, 2].map((value) => <ActionMenuItem key={value} onClick={() => { controller.setScale({ kind: "numeric", value }); onShowPdf(); }}>{value * 100}%</ActionMenuItem>)}
    </ActionMenu>
    <div className="pdf-view-search">
      <ControlHint content="搜索 PDF（Cmd/Ctrl+F）"><button ref={searchTrigger} type="button" aria-label="搜索 PDF" aria-expanded={open} onClick={() => { onShowPdf(); setOpen(!open); if (open) controller.closeFind(); }}><Search size={16} /></button></ControlHint>
      {open && <Popover store={searchPopover} fixed modal={false} hideOnInteractOutside={false} hideOnEscape={false} autoFocusOnHide={false}
        initialFocus={searchRef} getAnchorRect={() => searchTrigger.current?.getBoundingClientRect() ?? null} gutter={7}
        onKeyDown={event => handleShortcut(event.nativeEvent)} className="pdf-view-search-popover" role="search" aria-label="搜索 PDF 内容">
        <input ref={searchRef} aria-label="PDF 搜索文字" value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={(event) => { if (!event.nativeEvent.isComposing && event.key === "Enter") { event.preventDefault(); find(event.shiftKey); } }} />
        <button aria-label="上一个匹配项" type="button" disabled={!query} onClick={() => find(true)}><ChevronUp size={16} /></button>
        <button aria-label="下一个匹配项" type="button" disabled={!query} onClick={() => find()}><ChevronDown size={16} /></button>
        <button aria-label="关闭 PDF 搜索" type="button" onClick={close}><X size={16} /></button>
        <div className="pdf-view-search-options">
          <label><input type="checkbox" checked={caseSensitive} onChange={(event) => setCaseSensitive(event.target.checked)} />区分大小写</label>
          <label><input type="checkbox" checked={entireWord} onChange={(event) => setEntireWord(event.target.checked)} />全词</label>
          <label><input type="checkbox" checked={highlightAll} onChange={(event) => setHighlightAll(event.target.checked)} />全部匹配</label>
          <span aria-live="polite">{state.findCurrent} / {state.findTotal}</span>
        </div>
      </Popover>}
    </div>
  </div>;
}

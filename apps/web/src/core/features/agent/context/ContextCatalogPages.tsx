import {useLayoutEffect, useRef, useState, type CSSProperties} from "react";
import {ChevronDown, ChevronUp} from "lucide-react";
import {agentContextKey, type AgentContextSelection, type AgentContextState} from "@research-assistant/core/agent";
import {ControlHint} from "../../../components/ControlHint";
import {useScrollPageNavigation} from "../../../components/scroll/useScrollPageNavigation";
import {ContextCatalogRow} from "./ContextCatalogRow";
import {selectedCatalogItem} from "./contextCatalogSelection";

const DEFAULT_PAGE_SIZE = 8;
const ROW_MIN_HEIGHT = 35;
const PAGE_ANIMATION_MS = 280;

/** Page replacement keeps context choices in their conversation owner. */
export function ContextCatalogPages({rows, state, disabled, onChange}: {
  readonly rows: readonly AgentContextSelection[];
  readonly state: AgentContextState;
  readonly disabled: boolean;
  readonly onChange: (item: AgentContextSelection, remove: boolean) => void;
}): React.JSX.Element {
  const [page, setPage] = useState(0), [direction, setDirection] = useState<-1 | 0 | 1>(0);
  const [pageHeight, setPageHeight] = useState(DEFAULT_PAGE_SIZE * ROW_MIN_HEIGHT);
  const [outgoing, setOutgoing] = useState<{items: readonly AgentContextSelection[]; direction: -1 | 1; scrollTop: number} | null>(null);
  const stage = useRef<HTMLDivElement>(null), viewport = useRef<HTMLDivElement>(null), lockedUntil = useRef(0);
  const pageSize = Math.max(1, Math.floor(pageHeight / ROW_MIN_HEIGHT));
  const pages = Math.max(1, Math.ceil(rows.length / pageSize));
  const current = Math.min(page, pages - 1);
  // Balance the last page too, rather than leaving a single row in a tall viewport.
  const items = rows.slice(Math.floor(current * rows.length / pages), Math.floor((current + 1) * rows.length / pages));
  useLayoutEffect(() => {
    const node = stage.current;
    if (!node) return;
    const observer = new ResizeObserver(entries => {
      const height = entries[0]?.contentRect.height ?? 0;
      if (height > 0) setPageHeight(height);
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  function turn(delta: -1 | 1): void {
    const next = Math.max(0, Math.min(pages - 1, current + delta));
    if (next === current || performance.now() < lockedUntil.current) return;
    const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    lockedUntil.current = performance.now() + (reduced ? 0 : PAGE_ANIMATION_MS);
    setOutgoing(reduced ? null : {items, direction: delta, scrollTop: viewport.current?.scrollTop ?? 0});
    setDirection(delta); setPage(next);
    const focused = document.activeElement;
    if (focused instanceof HTMLElement && (viewport.current?.contains(focused)
      || (next === 0 && focused.getAttribute("aria-label") === "上一页上下文")
      || (next === pages - 1 && focused.getAttribute("aria-label") === "下一页上下文"))) viewport.current?.focus({preventScroll: true});
  }
  useScrollPageNavigation(viewport, turn);
  useLayoutEffect(() => {
    if (page !== current) setPage(current);
    const node = viewport.current;
    if (node) node.scrollTop = 0;
  }, [current, page, direction, pageSize]);
  function renderRows(values: readonly AgentContextSelection[], exiting = false): React.JSX.Element[] {
    return values.map(item => {
      const selected = selectedCatalogItem(item, state);
      return <ContextCatalogRow key={agentContextKey(item.resource)} item={item} {...(selected ? {selected} : {})}
        disabled={disabled || exiting} onChange={next => {if (!exiting) onChange(next ?? item, next === null);}}/>;
    });
  }
  return <div className="context-catalog-pager" style={{"--context-catalog-count": Math.max(2, rows.length)} as CSSProperties}>
    <div className="context-catalog-page-edge">
      {current > 0 && <ControlHint content="上一页，也可向上滚动" shortcut="PageUp"><button type="button" aria-label="上一页上下文" onClick={() => turn(-1)}><ChevronUp aria-hidden="true" size={17}/></button></ControlHint>}
    </div>
    <div ref={stage} className="context-catalog-stage" style={{"--context-page-distance": `${pageHeight}px`} as CSSProperties}>
      {outgoing && <div className="context-catalog-outgoing" inert aria-hidden="true" data-direction={outgoing.direction}
        onAnimationEnd={event => {if (event.target === event.currentTarget) setOutgoing(null);}}>
        <ul className="context-catalog context-catalog-page" style={{transform: `translateY(-${outgoing.scrollTop}px)`}}>{renderRows(outgoing.items, true)}</ul>
      </div>}
      <div ref={viewport} className="context-catalog-viewport" role="region" aria-label="上下文分页内容" tabIndex={0}
      onKeyDown={event => {
        if (event.target !== event.currentTarget || (event.key !== "PageDown" && event.key !== "PageUp")) return;
        const node = event.currentTarget, delta = event.key === "PageDown" ? 1 : -1;
        if (delta > 0 ? node.scrollHeight - node.clientHeight - node.scrollTop > 1 : node.scrollTop > 1) return;
        event.preventDefault(); turn(delta);
      }}>
      <ul key={current} className="context-catalog context-catalog-page" aria-label="上下文对象" data-page={current + 1} data-direction={direction}>
        {renderRows(items)}
      </ul>
      {rows.length === 0 && <p className="context-catalog-empty">没有匹配的上下文对象</p>}
      </div>
    </div>
    <div className="context-catalog-page-edge">
      {current < pages - 1 && <ControlHint content="下一页，也可向下滚动" shortcut="PageDown"><button type="button" aria-label="下一页上下文" onClick={() => turn(1)}><ChevronDown aria-hidden="true" size={17}/></button></ControlHint>}
    </div>
    <span className="context-catalog-status" role="status" aria-live="polite">第 {current + 1} / {pages} 页 · {rows.length} 项</span>
  </div>;
}

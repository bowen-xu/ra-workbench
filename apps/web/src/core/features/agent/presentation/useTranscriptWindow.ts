import {useLayoutEffect, useMemo, useRef, useState} from "react";
import {listWindowOffsets, listWindowRange} from "../../../public/layout";
import type { AgentMessageSearchTarget } from "../../../public/agentPresentation/useMessageSearchTarget";

interface TranscriptRow {readonly id: string; readonly messages: readonly {readonly id: string}[]}
interface PositionAnchor {readonly bottom: boolean; readonly id: string | undefined; readonly offset: number; readonly scrollTop: number}

/** Metadata and measured heights survive DOM recycling. Authority/history never
 * depends on this window. Reuse the same geometry as note/conversation lists. */
export function useTranscriptWindow<T extends TranscriptRow>(items: readonly T[], target?: AgentMessageSearchTarget | null, options: {readonly selector?: string; readonly estimate?: number; readonly fromEnd?: boolean; readonly enabled?: boolean} = {}) {
  const selector = options.selector ?? ".writing-agent-messages", estimate = options.estimate ?? 180;
  const fromEnd = options.fromEnd ?? true;
  const enabled = options.enabled ?? true;
  const body = useRef<HTMLDivElement>(null), sizes = useRef(new Map<string, number>());
  const [range, setRange] = useState({first: fromEnd ? Math.max(0, items.length - 8) : 0, end: fromEnd ? items.length : Math.min(8, items.length), count: items.length, measured: false, pinned: [] as string[]});
  const [revision, remeasure] = useState(0);
  const keys = JSON.stringify(items.map(item => item.id));
  const ids = useMemo(() => JSON.parse(keys) as string[], [keys]);
  const offsets = useMemo(() => listWindowOffsets(ids, sizes.current, estimate), [ids, revision, estimate]);
  const targetIndex = target ? items.findIndex(item => target.turnId ? item.id === target.turnId
    : item.messages.some(message => message.id === target.messageId)) : -1;
  const targetKey = JSON.stringify(target ?? null), navigated = useRef<string | undefined>(undefined);
  const pendingAnchor = useRef<PositionAnchor | null>(null);
  const settledAnchor = useRef<PositionAnchor | null>(null);
  const clearAnchorFrame = useRef<number | undefined>(undefined);
  function capturePosition(pane: HTMLElement, element: HTMLElement): PositionAnchor | null {
    if (!pane.clientHeight || (pane.scrollHeight <= pane.clientHeight && pane.dataset.followBottom !== "true")) return null;
    const top = pane.getBoundingClientRect().top + pane.clientTop;
    // Estimates decide which rows to mount, never which message the user is
    // reading. An expanded row can span several estimated row boundaries.
    const row = [...element.querySelectorAll<HTMLElement>(":scope > [data-transcript-row]")].find(node => {
      const rect = node.getBoundingClientRect();
      return rect.height > 0 && rect.bottom > top && rect.top < top + pane.clientHeight;
    });
    return {bottom: pane.dataset.followBottom === undefined
      ? pane.scrollHeight - pane.clientHeight - pane.scrollTop <= 6 : pane.dataset.followBottom === "true",
      id: row?.dataset.transcriptRow, offset: row ? row.getBoundingClientRect().top - top : 0, scrollTop: pane.scrollTop};
  }
  function rememberPosition(pane: HTMLElement, element: HTMLElement) {
    if (!pane.clientHeight || pane.scrollHeight <= pane.clientHeight) return;
    pane.dataset.transcriptAnchoring = "true";
    if (clearAnchorFrame.current !== undefined) cancelAnimationFrame(clearAnchorFrame.current);
    pendingAnchor.current ??= settledAnchor.current ?? capturePosition(pane, element);
  }
  const latest = useRef({items, ids, offsets, targetIndex, targetKey});
  latest.current = {items, ids, offsets, targetIndex, targetKey};
  const update = useRef(() => {});
  update.current = () => {
    const element = body.current, pane = element?.closest<HTMLElement>(selector);
    if (!element || !pane || !pane.clientHeight) return;
    const top = pane.getBoundingClientRect().top - element.getBoundingClientRect().top;
    const next = listWindowRange(latest.current.offsets, top, pane.clientHeight);
    const selection = element.ownerDocument.getSelection();
    const active = element.ownerDocument.activeElement;
    const pinned = Array.from(element.querySelectorAll<HTMLElement>(":scope > [data-transcript-row]")).flatMap(row => {
      const ownsFocus = active && active !== element.ownerDocument.body && row.contains(active);
      const ownsSelection = selection && !selection.isCollapsed && (row.contains(selection.anchorNode) || row.contains(selection.focusNode));
      return ownsFocus || ownsSelection || row.querySelector('[aria-expanded="true"]:not(details), details[open] summary:focus')
        ? [row.dataset.transcriptRow!] : [];
    });
    if (!(range.measured && range.first === next.first && range.end === next.end && range.count === latest.current.items.length && range.pinned.join() === pinned.join())) {
      rememberPosition(pane, element);
      setRange({...next, count: latest.current.items.length, measured: true, pinned});
    }
  };
  useLayoutEffect(() => {
    const element = body.current, pane = element?.closest<HTMLElement>(selector);
    if (!enabled || !element || !pane) return;
    let frame: number | undefined;
    const schedule = () => {if (frame === undefined) frame = requestAnimationFrame(() => {frame = undefined; update.current();});};
    const invalidate = () => {
      pendingAnchor.current = null; settledAnchor.current = null;
      delete pane.dataset.transcriptAnchoring;
    };
    const onInput = (event: Event) => {
      if (event.target instanceof Node && pane.contains(event.target)) {
        invalidate();
        // Rebase outer windows too when input is inside a nested viewport.
        // The position guard below rejects this snapshot if the input then
        // moves this pane before the next layout effect.
        settledAnchor.current = capturePosition(pane, element);
      }
    };
    const onScroll = () => {
      const saved = pendingAnchor.current ?? settledAnchor.current;
      const limit = Math.max(0, pane.scrollHeight - pane.clientHeight);
      // User / external scrolls replace the snapshot. A height shrink can also
      // clamp scrollTop; retain its anchor so the next layout can compensate.
      const clamped = saved && saved.scrollTop > limit && Math.abs(pane.scrollTop - limit) <= 1;
      if (saved && Math.abs(saved.scrollTop - pane.scrollTop) > 1 && !clamped) invalidate();
      if (!clamped) settledAnchor.current = capturePosition(pane, element);
      schedule();
    };
    let width = pane.clientWidth;
    const resize = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(() => {
      rememberPosition(pane, element);
      if (width !== pane.clientWidth) {width = pane.clientWidth; sizes.current.clear();}
      remeasure(value => value + 1); schedule();
    });
    resize?.observe(pane); resize?.observe(element);
    const mutations = new MutationObserver(schedule);
    mutations.observe(element, {attributes: true, subtree: true, attributeFilter: ["aria-expanded", "open"]});
    pane.addEventListener("scroll", onScroll, {passive: true});
    // Observe intent before the shared wheel router stops propagation at the
    // outer pane. Nested input must invalidate both inner and outer snapshots.
    const inputEvents = ["wheel", "pointerdown", "touchstart", "touchmove", "keydown"];
    for (const type of inputEvents) element.ownerDocument.addEventListener(type, onInput, {capture: true, passive: true});
    pane.addEventListener("writing-agent-preserve-position", invalidate);
    element.addEventListener("focusin", schedule); element.addEventListener("focusout", schedule);
    element.ownerDocument.addEventListener("selectionchange", schedule);
    window.addEventListener("resize", schedule); schedule();
    return () => {if (frame !== undefined) cancelAnimationFrame(frame); if (clearAnchorFrame.current !== undefined) cancelAnimationFrame(clearAnchorFrame.current);
      invalidate(); resize?.disconnect(); mutations.disconnect();
      pane.removeEventListener("scroll", onScroll);
      for (const type of inputEvents) element.ownerDocument.removeEventListener(type, onInput, true);
      pane.removeEventListener("writing-agent-preserve-position", invalidate);
      element.removeEventListener("focusin", schedule); element.removeEventListener("focusout", schedule);
      element.ownerDocument.removeEventListener("selectionchange", schedule); window.removeEventListener("resize", schedule);};
  }, [selector, enabled]);
  useLayoutEffect(() => {
    const element = body.current, pane = element?.closest<HTMLElement>(selector);
    if (!enabled || !element || !pane || !pane.clientHeight) return;
    const current = latest.current;
    const restore = pendingAnchor.current ?? settledAnchor.current;
    pendingAnchor.current = null;
    settledAnchor.current = null;
    const limit = Math.max(0, pane.scrollHeight - pane.clientHeight);
    const samePosition = restore && (Math.abs(restore.scrollTop - pane.scrollTop) <= 1
      || (restore.scrollTop > limit && Math.abs(pane.scrollTop - limit) <= 1));
    if (restore && samePosition) {
      const row = [...element.querySelectorAll<HTMLElement>(":scope > [data-transcript-row]")]
        .find(node => node.dataset.transcriptRow === restore.id);
      const rect = row?.getBoundingClientRect();
      const next = restore.bottom && pane.dataset.followBottom !== "false" ? Math.max(0, pane.scrollHeight - pane.clientHeight)
        : rect && rect.height > 0 ? pane.scrollTop + rect.top - pane.getBoundingClientRect().top - pane.clientTop - restore.offset : pane.scrollTop;
      if (Math.abs(next - pane.scrollTop) > .5) pane.scrollTop = next;
    }
    settledAnchor.current = capturePosition(pane, element);
    let changed = false;
    for (const row of element.querySelectorAll<HTMLElement>(":scope > [data-transcript-row]")) {
      const id = row.dataset.transcriptRow!, height = row.getBoundingClientRect().height;
      if (height > 0 && sizes.current.get(id) !== height) {sizes.current.set(id, height); changed = true;}
    }
    if (changed) {
      rememberPosition(pane, element);
      remeasure(value => value + 1);
    } else {
      update.current();
      if (!pendingAnchor.current) {
        if (clearAnchorFrame.current !== undefined) cancelAnimationFrame(clearAnchorFrame.current);
        clearAnchorFrame.current = requestAnimationFrame(() => {
          clearAnchorFrame.current = requestAnimationFrame(() => {delete pane.dataset.transcriptAnchoring; clearAnchorFrame.current = undefined;});
        });
      }
    }
    const retained = new Set(current.ids);
    for (const id of sizes.current.keys()) if (!retained.has(id)) sizes.current.delete(id);
  }, [items, offsets, range, selector, estimate, enabled]);
  useLayoutEffect(() => {
    const element = body.current, pane = element?.closest<HTMLElement>(selector);
    if (!element || !pane || targetIndex < 0 || navigated.current === targetKey) return;
    navigated.current = targetKey;
    pane.dispatchEvent(new Event("writing-agent-preserve-position"));
    if (pane.clientHeight) pane.scrollTop += offsets[targetIndex]! + element.getBoundingClientRect().top - pane.getBoundingClientRect().top;
    update.current();
  }, [targetKey, targetIndex, offsets, selector]);
  const end = range.end >= range.count ? items.length : Math.min(items.length, range.end);
  const first = range.end >= range.count ? Math.max(0, end - (range.measured ? Math.max(1, range.end - range.first) : 8)) : range.first;
  const indices = [...new Set([
    ...Array.from({length: Math.max(0, end - first)}, (_, index) => first + index),
    ...range.pinned.map(id => ids.indexOf(id)).filter(index => index >= 0),
    // Retain the physical anchor across a range update until its measured
    // position has been restored. Never substitute an estimated, unrelated row.
    ...[pendingAnchor.current?.id, settledAnchor.current?.id].flatMap(id => id === undefined ? [] : ids.indexOf(id) >= 0 ? [ids.indexOf(id)] : []),
    ...(targetIndex >= 0 ? [targetIndex] : []),
  ])].sort((a, b) => a - b);
  let previous = 0;
  const rows = indices.flatMap(index => {
    const item = items[index]; if (!item) return [];
    const gap = offsets[index]! - previous; previous = offsets[index + 1]!;
    return [{item, index, gap}];
  });
  return {body, rows, bottom: offsets.at(-1)! - previous};
}

import { useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import type { AgentConversationSummary } from "@research-assistant/core/agent";
import { listWindowOffsets, listWindowRange } from "../../components/layout/listWindowGeometry";
import { useVisibleLayoutMeasurement } from "../../components/layout/useVisibleLayoutMeasurement";

/** Conversation metadata stays in memory while off-screen rows are unmounted. */
export function useConversationListWindow(items: readonly AgentConversationSummary[], root: RefObject<HTMLElement | null>, pinnedId?: string) {
  const body = useRef<HTMLDivElement>(null), sizes = useRef(new Map<string, number>());
  const [range, setRange] = useState({first: 0, end: 50, pinned: [] as number[]});
  const [revision, setRevision] = useState(0);
  const offsets = useMemo(() => listWindowOffsets(items.map(item => item.id), sizes.current, 30), [items, revision]);
  const latest = useRef({items, offsets, pinnedId}); latest.current = {items, offsets, pinnedId};
  const updateRef = useRef(() => {});
  updateRef.current = () => {
    const pane = root.current, content = body.current;
    if (!pane || !content || !pane.clientHeight) return;
    const top = pane.getBoundingClientRect().top - content.getBoundingClientRect().top;
    const next = listWindowRange(latest.current.offsets, top, pane.clientHeight);
    const ids = Array.from(content.querySelectorAll<HTMLElement>(":focus, [aria-expanded='true']"))
      .flatMap(control => control.closest<HTMLElement>("[data-conversation-id]")?.dataset.conversationId ?? []);
    if (latest.current.pinnedId) ids.push(latest.current.pinnedId);
    const pinned = [...new Set(ids.map(id => latest.current.items.findIndex(item => item.id === id))
      .filter(index => index >= 0 && (index < next.first || index >= next.end)))].sort((a, b) => a - b);
    setRange(previous => previous.first === next.first && previous.end === next.end && previous.pinned.join() === pinned.join()
      ? previous : {...next, pinned});
  };
  useVisibleLayoutMeasurement(() => root.current, () => updateRef.current());
  useLayoutEffect(() => {
    const pane = root.current, content = body.current;
    if (!pane || !content) return;
    let frame: number | undefined;
    const schedule = () => {if (frame === undefined) frame = requestAnimationFrame(() => {frame = undefined; updateRef.current();});};
    const resize = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(() => {setRevision(value => value + 1); schedule();});
    resize?.observe(pane); resize?.observe(content);
    const mutation = new MutationObserver(schedule);
    mutation.observe(content, {attributes: true, subtree: true, attributeFilter: ["aria-expanded"]});
    pane.addEventListener("scroll", schedule, {passive: true});
    content.addEventListener("focusin", schedule); content.addEventListener("focusout", schedule);
    window.addEventListener("resize", schedule); schedule();
    return () => {if (frame !== undefined) cancelAnimationFrame(frame); resize?.disconnect(); mutation.disconnect();
      pane.removeEventListener("scroll", schedule); content.removeEventListener("focusin", schedule); content.removeEventListener("focusout", schedule); window.removeEventListener("resize", schedule);};
  }, [root]);
  useLayoutEffect(() => {
    const ids = new Set(items.map(item => item.id));
    for (const id of sizes.current.keys()) if (!ids.has(id)) sizes.current.delete(id);
    let changed = false;
    for (const row of body.current?.querySelectorAll<HTMLElement>("[data-conversation-id]") ?? []) {
      const id = row.dataset.conversationId, height = row.getBoundingClientRect().height;
      if (id && height > 0 && sizes.current.get(id) !== height) {sizes.current.set(id, height); changed = true;}
    }
    if (changed) setRevision(value => value + 1); else updateRef.current();
  }, [items, range, revision, pinnedId]);
  return useMemo(() => {
    const indices = [...new Set([...Array.from({length: Math.max(0, Math.min(items.length, range.end) - range.first)}, (_, index) => range.first + index), ...range.pinned.filter(index => index < items.length)])].sort((a, b) => a - b);
    let previous = 0;
    const rows = indices.map(index => {const gap = offsets[index]! - previous; previous = offsets[index + 1]!; return {item: items[index]!, gap};});
    return {body, rows, bottom: offsets[offsets.length - 1]! - previous};
  }, [items, offsets, range]);
}

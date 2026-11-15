import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from "react";
import { notify } from "../../services/notifications";
import { reorderedRowIds, reorderRowOffsets, type ReorderRowGeometry } from "../reorderLayout";
import { cancelReorderMotion, type ReorderAxis } from "./reorderMotion";

export interface ReorderChange {
  readonly source: string;
  readonly target: string;
  readonly after: boolean;
  readonly order: readonly string[];
}
export type ReorderPlacement = "before" | "after" | "inside";
export interface ReorderDrop { readonly source: string; readonly target: string; readonly placement: ReorderPlacement }
export interface ReorderTreeItem { readonly parentId?: string; readonly depth: number; readonly canNest: boolean }
interface Options {
  readonly presentation?: "displacement" | "indicator";
  readonly treeItem?: (id: string) => ReorderTreeItem;
  readonly canDrop?: (drop: ReorderDrop) => boolean;
  readonly onDrop?: (drop: ReorderDrop) => void | Promise<void>;
  readonly ids: readonly string[];
  readonly axis?: ReorderAxis;
  readonly disabled?: boolean;
  readonly scrollContainer?: RefObject<HTMLElement | null>;
  readonly onReorder?: (change: ReorderChange) => void | boolean | Promise<void | boolean>;
}
interface Gesture {
  readonly id: string;
  readonly pointerId: number;
  readonly origin: number;
  readonly identity: string;
  readonly positions: readonly ReorderRowGeometry[];
  readonly scroll: HTMLElement | undefined;
  index: number;
  started: boolean;
  drop?: ReorderDrop | undefined;
}
interface Preview { readonly id: string; readonly target: string; readonly placement?: ReorderPlacement | undefined; readonly offsets: ReadonlyMap<string, number> }
interface OptimisticOrder { readonly base: string; readonly order: readonly string[] }

function scrollParent(element: HTMLElement | undefined, axis: ReorderAxis): HTMLElement | undefined {
  for (let parent = element?.parentElement; parent; parent = parent.parentElement) {
    const style = getComputedStyle(parent);
    if (/(auto|scroll)/u.test(axis === "horizontal" ? style.overflowX : style.overflowY)) return parent;
  }
  return undefined;
}

/** Single pointer lifecycle for horizontal tabs and vertical lists. Hosts own
 * data and save errors; optimistic ordering is visual only, never durable data.
 */
export function usePointerReorder(options: Options) {
  const latest = useRef(options); latest.current = options;
  const rows = useRef(new Map<string, HTMLElement>());
  const gesture = useRef<Gesture | undefined>(undefined);
  const suppressClick = useRef(false);
  const [preview, setPreview] = useState<Preview>();
  const [optimistic, setOptimistic] = useState<OptimisticOrder>();
  const [pending, setPending] = useState(false);
  const operation = useRef(0), alive = useRef(true);
  const axis = options.axis ?? "vertical";
  const identity = options.ids.join("\0");
  const order = optimistic?.base === identity ? optimistic.order : options.ids;
  const renderedOrder = useRef(order); renderedOrder.current = order;
  const drag = !options.disabled && gesture.current?.identity === identity ? preview : undefined;
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; operation.current++; };
  }, []);
  useEffect(() => {
    if (gesture.current) { suppressClick.current = gesture.current.started; gesture.current = undefined; setPreview(undefined); }
  }, [identity, options.disabled, axis]);
  useEffect(() => { if (optimistic && optimistic.base !== identity) setOptimistic(undefined); }, [identity, optimistic]);

  useEffect(() => {
    function cancel(): void {
      if (!gesture.current) return;
      suppressClick.current = gesture.current.started;
      gesture.current = undefined; setPreview(undefined);
    }
    function update(event: PointerEvent): void {
      const active = gesture.current;
      if (!active || active.pointerId !== event.pointerId || latest.current.disabled) return;
      const point = axis === "horizontal" ? event.clientX : event.clientY;
      if (!active.started && Math.abs(point - active.origin) < 6) return;
      active.started = true; suppressClick.current = true; event.preventDefault();
      if (active.scroll) {
        const bounds = active.scroll.getBoundingClientRect();
        if (axis === "horizontal") {
          if (point > bounds.right - 25) active.scroll.scrollLeft += 14;
          else if (point < bounds.left + 45) active.scroll.scrollLeft -= 14;
        } else {
          if (point > bounds.bottom - 25) active.scroll.scrollTop += 14;
          else if (point < bounds.top + 25) active.scroll.scrollTop -= 14;
        }
      }
      if (latest.current.presentation === "indicator") {
        const entries = active.positions.map(value => ({ id: value.id, rect: rows.current.get(value.id)?.getBoundingClientRect() })).filter(value => value.rect !== undefined);
        let entry = entries.find(value => point >= value.rect!.top && point <= value.rect!.bottom)
          ?? entries.reduce<typeof entries[number] | undefined>((best, value) => !best || Math.abs(point - value.rect!.top) < Math.abs(point - best.rect!.top) ? value : best, undefined);
        if (!entry) return;
        const fraction = (point - entry.rect!.top) / Math.max(1, entry.rect!.height);
        let target = entry.id, item = latest.current.treeItem?.(target);
        // Outdent into an ancestor's sibling list when the pointer enters its gutter.
        while (item?.parentId && event.clientX < entry.rect!.left + item.depth * 18 + 12) {
          target = item.parentId; item = latest.current.treeItem?.(target);
        }
        const placement: ReorderPlacement = target !== entry.id ? (fraction < .5 ? "before" : "after")
          : fraction > .25 && fraction < .75 && item?.canNest ? "inside" : fraction < .5 ? "before" : "after";
        const drop = { source: active.id, target, placement };
        active.drop = target !== active.id && (latest.current.canDrop?.(drop) ?? true) ? drop : undefined;
        setPreview({ id: active.id, target: active.drop?.target ?? active.id, placement: active.drop?.placement, offsets: new Map() });
        return;
      }
      const contentPoint = point + (axis === "horizontal" ? active.scroll?.scrollLeft ?? 0 : active.scroll?.scrollTop ?? 0);
      const boundary = (index: number): number => {
        const left = active.positions[index]!, right = active.positions[index + 1]!;
        return (left.top + left.height + right.top) / 2;
      };
      while (active.index < active.positions.length - 1 && contentPoint > boundary(active.index) + 4) active.index++;
      while (active.index > 0 && contentPoint < boundary(active.index - 1) - 4) active.index--;
      const target = active.positions[active.index]!.id;
      const sourceIndex = active.positions.findIndex(position => position.id === active.id);
      const ids = active.positions.map(position => position.id);
      const next = reorderedRowIds(ids, [active.id], target, active.index > sourceIndex ? "after" : "before");
      setPreview({ id: active.id, target, offsets: reorderRowOffsets(active.positions, next) });
    }
    function finish(event: PointerEvent): void {
      const active = gesture.current;
      if (!active || active.pointerId !== event.pointerId) return;
      if (event.type === "pointercancel" || latest.current.disabled) { cancel(); return; }
      update(event);
      gesture.current = undefined; setPreview(undefined);
      if (!active.started) return;
      window.setTimeout(() => { suppressClick.current = false; }, 0);
      if (latest.current.presentation === "indicator") {
        if (!active.drop) return;
        setPending(true);
        try { void Promise.resolve(latest.current.onDrop?.(active.drop)).catch(reason => {
          if (alive.current) notify.error(reason instanceof Error ? reason.message : String(reason));
        }).finally(() => { if (alive.current) setPending(false); }); }
        catch (reason) { setPending(false); notify.error(reason instanceof Error ? reason.message : String(reason)); }
        return;
      }
      const ids = active.positions.map(position => position.id);
      const target = ids[active.index]!;
      if (target === active.id) return;
      const after = active.index > ids.indexOf(active.id);
      const next = reorderedRowIds(ids, [active.id], target, after ? "after" : "before");
      const token = ++operation.current;
      setOptimistic({ base: latest.current.ids.join("\0"), order: next });
      function rejected(reason?: unknown): void {
        if (!alive.current || operation.current !== token) return;
        setOptimistic(undefined);
        if (reason !== undefined) notify.error(reason instanceof Error ? reason.message : "调整顺序失败，请重试。");
      }
      try {
        const result = latest.current.onReorder?.({ source: active.id, target, after, order: next });
        if (result && typeof result !== "boolean") {
          setPending(true);
          void result.then(accepted => { if (accepted === false) rejected(); }, rejected).finally(() => {
            if (alive.current && operation.current === token) setPending(false);
          });
        } else if (result === false) rejected();
      } catch (reason) { rejected(reason); }
    }
    function keydown(event: KeyboardEvent): void { if (event.key === "Escape" && gesture.current) { event.preventDefault(); cancel(); } }
    function lostCapture(event: PointerEvent): void { if (gesture.current?.pointerId === event.pointerId) cancel(); }
    window.addEventListener("pointermove", update); window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish); window.addEventListener("keydown", keydown); window.addEventListener("blur", cancel);
    window.addEventListener("lostpointercapture", lostCapture);
    return () => {
      window.removeEventListener("pointermove", update); window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish); window.removeEventListener("keydown", keydown); window.removeEventListener("blur", cancel);
      window.removeEventListener("lostpointercapture", lostCapture);
    };
  }, [axis]);
  return {
    rows, drag, order, pending,
    offsetFor(id: string): number { return drag?.offsets.get(id) ?? 0; },
    start(id: string, event: ReactPointerEvent<HTMLElement>): void {
      if (options.disabled || pending || event.button !== 0 || !event.isPrimary || gesture.current) return;
      cancelReorderMotion(rows.current.values());
      const scroll = options.scrollContainer?.current ?? scrollParent(rows.current.get(id), axis);
      const scrollOffset = axis === "horizontal" ? scroll?.scrollLeft ?? 0 : scroll?.scrollTop ?? 0;
      const positions = renderedOrder.current.flatMap(value => {
        const rect = rows.current.get(value)?.getBoundingClientRect();
        return rect ? [{ id: value, top: (axis === "horizontal" ? rect.left : rect.top) + scrollOffset, height: axis === "horizontal" ? rect.width : rect.height }] : [];
      });
      const index = positions.findIndex(position => position.id === id);
      if (index < 0) return;
      suppressClick.current = false;
      gesture.current = { id, pointerId: event.pointerId, origin: axis === "horizontal" ? event.clientX : event.clientY, identity, positions, scroll, index, started: false };
      if (options.presentation !== "indicator") event.currentTarget.setPointerCapture?.(event.pointerId);
    },
    consumeClick(): boolean { const value = suppressClick.current; suppressClick.current = false; return value; },
  };
}

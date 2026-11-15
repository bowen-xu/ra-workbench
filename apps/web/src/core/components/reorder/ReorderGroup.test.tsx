// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { ReorderGroup, ReorderItem } from "./ReorderGroup";
import { usePointerReorder, type ReorderChange } from "./usePointerReorder";
import type { ReorderAxis } from "./reorderMotion";

vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
let dispose: (() => Promise<void>) | undefined;
afterEach(async () => { await dispose?.(); document.body.innerHTML = ""; vi.restoreAllMocks(); });

async function setup(axis: ReorderAxis, delayed = false, presentation: "displacement" | "indicator" = "displacement") {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); dispose = () => act(async () => root.unmount());
  let replace!: (ids: string[]) => void, lock!: (value: boolean) => void;
  let complete: ((accepted: boolean) => void) | undefined;
  const commit = vi.fn();
  function Shell() {
    const [ids, setIds] = useState(["a", "b", "c"]), [disabled, setDisabled] = useState(false);
    replace = setIds; lock = setDisabled;
    const reorder = usePointerReorder({ ids, axis, disabled, presentation, treeItem: id => ({ depth: id === "c" ? 1 : 0, ...(id === "c" ? { parentId: "b" } : {}), canNest: true }), onDrop: drop => { commit(drop); }, onReorder(change: ReorderChange) {
      commit(change);
      if (!delayed) { setIds([...change.order]); return true; }
      return new Promise<boolean>(resolve => { complete = accepted => { if (accepted) setIds([...change.order]); resolve(accepted); }; });
    } });
    return <ReorderGroup motion={presentation === "indicator" ? "none" : "default"} order={reorder.order} axis={axis} dragging={!!reorder.drag}>
      {reorder.order.map(id => <ReorderItem key={id} itemId={id} axis={axis} offset={reorder.offsetFor(id)} dragging={reorder.drag?.id === id}
        ref={element => { if (element) reorder.rows.current.set(id, element); else reorder.rows.current.delete(id); }}>
        <button type="button" onPointerDown={event => reorder.start(id, event)}>{id}</button>
      </ReorderItem>)}
    </ReorderGroup>;
  }
  await act(async () => root.render(<Shell />));
  const group = host.querySelector<HTMLElement>(".reorder-group")!;
  const rows = new Map([...group.children].map(row => [(row as HTMLElement).dataset.reorderId!, row as HTMLElement]));
  const visible = new Map<string, number>();
  const sizes = new Map([["a", 80], ["b", 140], ["c", 100]]);
  const motion = new Map<string, { animate: ReturnType<typeof vi.fn>; cancel: ReturnType<typeof vi.fn> }>();
  for (const [id, row] of rows) {
    const cancel = vi.fn(), animate = vi.fn(() => ({ cancel }));
    Object.defineProperty(row, "animate", { value: animate }); motion.set(id, { animate, cancel });
    vi.spyOn(row, "getBoundingClientRect").mockImplementation(() => {
      let position = 36;
      for (const sibling of group.children) {
        if (sibling === row) break;
        position += sizes.get((sibling as HTMLElement).dataset.reorderId!)! + 6;
      }
      if (group.dataset.reordering && visible.has(id)) position = visible.get(id)!;
      return axis === "horizontal" ? new DOMRect(position, 0, sizes.get(id)!, 26) : new DOMRect(0, position, 300, sizes.get(id)!);
    });
  }
  const pointer = (target: Element | Window, type: string, position: number, pointerId = 1, pointX = 10) => act(async () => {
    target.dispatchEvent(new PointerEvent(type, { isPrimary: true, button: 0, pointerId, clientX: axis === "horizontal" ? position : pointX,
      clientY: axis === "vertical" ? position : 10, bubbles: true, cancelable: true }));
  });
  const order = () => [...group.children].map(row => (row as HTMLElement).dataset.reorderId);
  const offset = (id: string) => rows.get(id)!.style.getPropertyValue(axis === "horizontal" ? "--reorder-x" : "--reorder-y");
  return { group, rows, visible, motion, commit, pointer, order, offset,
    down: (id = "a", position = 60) => pointer(rows.get(id)!.querySelector("button")!, "pointerdown", position),
    complete: (accepted: boolean) => act(async () => complete!(accepted)),
    replace: (ids: string[]) => act(async () => replace(ids)), lock: () => act(async () => lock(true)) };
}

it.each(["horizontal", "vertical"] as const)("%s: switches at boundaries with hysteresis and preserves keyed rows on a quick release", async axis => {
  const f = await setup(axis); await f.down();
  await f.pointer(window, "pointermove", 123); expect(f.offset("a")).toBe("0px");
  await f.pointer(window, "pointermove", 124); expect(f.offset("a")).toBe("146px");
  await f.pointer(window, "pointermove", 115); expect(f.offset("a")).toBe("146px");
  await f.pointer(window, "pointermove", 114); expect(f.offset("a")).toBe("0px");
  await f.pointer(window, "pointermove", 124);
  f.visible.set("a", 100); f.visible.set("b", 90);
  await f.pointer(window, "pointerup", 124);
  expect(f.order()).toEqual(["b", "a", "c"]);
  expect(f.group.children[1]).toBe(f.rows.get("a"));
  expect(f.commit).toHaveBeenCalledOnce();
  const translate = axis === "horizontal" ? "translateX" : "translateY";
  expect(f.motion.get("a")!.animate).toHaveBeenCalledWith([{ transform: `${translate}(-82px)` }, { transform: `${translate}(0px)` }], expect.objectContaining({ duration: 180 }));
  expect(f.motion.get("b")!.animate).toHaveBeenCalledWith([{ transform: `${translate}(54px)` }, { transform: `${translate}(0px)` }], expect.objectContaining({ duration: 180 }));
  expect(f.group.hasAttribute("data-reordering")).toBe(false);
  expect(f.offset("a")).toBe("0px");
});

it.each([true, false])("holds visual order while saving and reconciles acceptance=%s without a second submission", async accepted => {
  const f = await setup("vertical", true); await f.down(); await f.pointer(window, "pointermove", 124); await f.pointer(window, "pointerup", 124);
  expect(f.order()).toEqual(["b", "a", "c"]);
  await f.down("b", 60); await f.pointer(window, "pointermove", 350); await f.pointer(window, "pointerup", 350);
  expect(f.commit).toHaveBeenCalledOnce(); expect(f.order()).toEqual(["b", "a", "c"]);
  const calls = f.motion.get("a")!.animate.mock.calls.length;
  await f.complete(accepted);
  expect(f.order()).toEqual(accepted ? ["b", "a", "c"] : ["a", "b", "c"]);
  expect(f.commit).toHaveBeenCalledOnce();
  if (accepted) expect(f.motion.get("a")!.animate.mock.calls.length).toBe(calls);
  else expect(f.motion.get("a")!.animate).toHaveBeenLastCalledWith([{ transform: "translateY(146px)" }, { transform: "translateY(0px)" }], expect.anything());
});

it.each(["escape", "pointercancel", "lostpointercapture", "lock", "remove"])("cancels on %s without submitting an obsolete move", async reason => {
  const f = await setup("vertical"); await f.down(); await f.pointer(window, "pointermove", 124);
  if (reason === "escape") await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
  else if (reason === "pointercancel") await f.pointer(window, "pointercancel", 124);
  else if (reason === "lostpointercapture") await f.pointer(window, "lostpointercapture", 124);
  else if (reason === "lock") await f.lock();
  else await f.replace(["a", "b"]);
  await f.pointer(window, "pointerup", 124);
  expect(f.commit).not.toHaveBeenCalled(); expect(f.group.dataset.reordering).toBeUndefined(); expect(f.offset("a")).toBe("0px");
});

it("honors reduced motion on a vertical commit", async () => {
  const f = await setup("vertical"); f.group.style.setProperty("--reorder-motion-duration", "0ms");
  await f.down(); await f.pointer(window, "pointermove", 124); await f.pointer(window, "pointerup", 124);
  expect(f.order()).toEqual(["b", "a", "c"]);
  expect([...f.motion.values()].every(value => value.animate.mock.calls.length === 0)).toBe(true);
});

it("indicator mode keeps every row stationary, selects edge/inside targets and outdents through the shared tree geometry", async () => {
  const f = await setup("vertical", false, "indicator");
  await f.down(); await f.pointer(window, "pointermove", 190, 1, 100);
  expect(f.offset("a")).toBe("0px"); expect(f.offset("b")).toBe("0px");
  await f.pointer(window, "pointerup", 190, 1, 100);
  expect(f.commit).toHaveBeenLastCalledWith({ source: "a", target: "b", placement: "inside" });
  await f.down(); await f.pointer(window, "pointerup", 123, 1, 100);
  expect(f.commit).toHaveBeenLastCalledWith({ source: "a", target: "b", placement: "before" });
  await f.down(); await f.pointer(window, "pointerup", 330, 1, 10);
  expect(f.commit).toHaveBeenLastCalledWith({ source: "a", target: "b", placement: "after" });
  expect(f.order()).toEqual(["a", "b", "c"]);
  expect(f.motion.get("a")!.animate).not.toHaveBeenCalled();
});

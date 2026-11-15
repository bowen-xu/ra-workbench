/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { WritingInputQueue } from "./WritingInputQueue";
it("reorders by identity and locks queue actions when an edit is uncertain", async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  const onEdit = vi.fn(async () => true); const onRetry = vi.fn();
  const state = { queue: { schemaVersion: 1 as const, conversationId: "conversation", manuscriptId: "manuscript", revision: 3, paused: false,
    preparingInputId: null, inputs: [{ inputId: "a", text: "First" }, { inputId: "b", text: "Second" }] },
  editing: false, loading: false, uncertain: false, error: null };
  try {
    await act(async () => root.render(<WritingInputQueue state={state} disabled={false} onEdit={onEdit} onRetry={onRetry} onReload={() => undefined} />));
    const secondItem = container.querySelectorAll<HTMLLIElement>("li")[1];
    const rows = [...container.querySelectorAll<HTMLLIElement>("li")];
    rows.forEach((row, index) => vi.spyOn(row, "getBoundingClientRect").mockReturnValue({ top: index * 40, height: 40 } as DOMRect));
    await act(async () => secondItem!.querySelector<HTMLButtonElement>('[aria-label="拖动调整第 2 条消息顺序"]')!
      .dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, isPrimary: true, pointerId: 1, clientY: 60 })));
    await act(async () => window.dispatchEvent(new PointerEvent("pointermove", { isPrimary: true, pointerId: 1, clientY: 50 })));
    const firstHandle = rows[0]!.querySelector<HTMLButtonElement>('.agent-input-queue-drag-handle')!;
    expect(firstHandle.disabled).toBe(true);
    expect(container.querySelector('.agent-input-queue-card')?.classList.contains('is-reordering')).toBe(true);
    await act(async () => firstHandle.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, isPrimary: true, pointerId: 2, clientY: 5 })));
    expect(secondItem!.classList.contains('is-dragging')).toBe(true);
    expect(rows[0]!.classList.contains('is-dragging')).toBe(false);
    // Cross the row boundary at 40 by >4px without reaching the first center at 20.
    await act(async () => window.dispatchEvent(new PointerEvent("pointermove", { isPrimary: true, pointerId: 1, clientY: 35 })));
    await act(async () => window.dispatchEvent(new PointerEvent("pointerup", { isPrimary: true, pointerId: 1, clientY: 35 })));
    expect(onEdit).toHaveBeenCalledWith({ type: "reorder", inputIds: ["b", "a"] });
    expect(firstHandle.disabled).toBe(false);
    expect(container.querySelector('.agent-input-queue-card')?.classList.contains('is-reordering')).toBe(false);
    await act(async () => root.render(<WritingInputQueue state={{ ...state, uncertain: true, error: "Unknown result" }} disabled={false}
      onEdit={onEdit} onRetry={onRetry} onReload={() => undefined} />));
    const buttons = [...container.querySelectorAll("button")];
    expect(buttons.some((button) => button.textContent === "立即发送队首")).toBe(false);
    expect(buttons.find((button) => button.getAttribute("aria-label") === "移除第 1 条消息")?.disabled).toBe(true);
    expect(buttons.some(button => button.textContent === "刷新队列" || button.textContent === "按原请求重试")).toBe(false);
    await act(async () => buttons.find((button) => button.getAttribute("aria-label") === "重试队列同步")!.click());
    expect(onRetry).toHaveBeenCalledOnce();
  } finally { await act(async () => root.unmount()); container.remove(); }
});

it("shows persisted entry provenance and explicitly labels legacy unknown origins",async()=>{
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container=document.createElement("div"),root=createRoot(container);
  const resourceId="11111111-1111-4111-8111-111111111111";
  const state={queue:{schemaVersion:1 as const,conversationId:"conversation",manuscriptId:"current",revision:1,paused:false,preparingInputId:null,
    inputs:[{inputId:"one",text:"reading message",origin:{kind:"readingNote" as const,resourceId}},{inputId:"two",text:"legacy"}]},
    editing:false,loading:false,uncertain:false,error:null};
  try{
    await act(async()=>root.render(<WritingInputQueue state={state} disabled={false} onEdit={async()=>true} onRetry={()=>undefined} onReload={()=>undefined}/>));
    const items = [...container.querySelectorAll<HTMLLIElement>("li")];
    expect(items[0]?.querySelector("p")?.getAttribute("aria-description")).toBe(`入队来源：阅读笔记 · ${resourceId}`);
    expect(items[1]?.querySelector("p")?.getAttribute("aria-description")).toBe("入队来源：未记录");
    expect(container.textContent).toContain("reading message");
  }finally{await act(async()=>root.unmount());}
});

it("does not expose a manual queue-head send action", async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div"), root = createRoot(container);
  const state = { queue: { schemaVersion: 1 as const, conversationId: "conversation", manuscriptId: "current", revision: 1, paused: false, preparingInputId: null,
    inputs: [{ inputId: "one", text: "automatic follow-up" }] }, editing: false, loading: false, uncertain: false, error: null };
  try {
    await act(async () => root.render(<WritingInputQueue state={state} disabled={true}
      onEdit={async () => true} onRetry={() => undefined} onReload={() => undefined} />));
    expect([...container.querySelectorAll("button")].some((button) => button.textContent === "立即发送队首")).toBe(false);
  } finally { await act(async () => root.unmount()); container.remove(); }
});

it("uses only the compact message menu while retaining inline removal", async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div"), root = createRoot(container);
  document.body.append(container);
  const onEdit = vi.fn(async () => true);
  const state = { queue: { schemaVersion: 1 as const, conversationId: "conversation", manuscriptId: "current", revision: 1, paused: false, preparingInputId: null,
    inputs: [{ inputId: "one", text: "follow-up" }, { inputId: "two", text: "next follow-up" }] }, editing: false, loading: false, uncertain: false, error: null };
  try {
    await act(async () => root.render(<WritingInputQueue state={state} disabled={false}
      onEdit={onEdit} onRetry={() => undefined} onReload={() => undefined} />));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="第 1 条消息菜单"]')!.click());
    const menu = document.querySelector('[role="menu"]')!;
    expect([...menu.querySelectorAll("button")].map(button => button.textContent)).toEqual(["编辑消息"]);
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="第 2 条消息菜单"]')!.click());
    expect([...document.querySelectorAll<HTMLElement>('[role="menu"]')].at(-1)!.textContent).toBe("编辑消息");
    await act(async () => root.render(<WritingInputQueue state={{ ...state, queue: { ...state.queue, paused: true } }} disabled={false}
      onEdit={onEdit} onRetry={() => undefined} onReload={() => undefined} />));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="第 1 条消息菜单"]')!.click());
    expect(document.querySelector('[role="menu"]')!.textContent).toBe("编辑消息");
    expect(onEdit).not.toHaveBeenCalled();
    await act(async () => root.render(<WritingInputQueue state={{ ...state, queue: { ...state.queue, paused: true }, uncertain: true }} disabled={false}
      onEdit={onEdit} onRetry={() => undefined} onReload={() => undefined} />));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="第 1 条消息菜单"]')!.click());
    expect(document.querySelector('[role="menu"]')!.textContent).toBe("编辑消息");
    await act(async () => root.render(<WritingInputQueue state={state} disabled={false}
      onEdit={onEdit} onRetry={() => undefined} onReload={() => undefined} />));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="移除第 1 条消息"]')!.click());
    expect(onEdit).toHaveBeenCalledWith({ type: "remove", inputId: "one" });
  } finally { await act(async () => root.unmount()); container.remove(); }
});

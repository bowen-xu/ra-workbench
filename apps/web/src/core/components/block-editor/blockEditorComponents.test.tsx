// @vitest-environment happy-dom
import {act, createRef} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {blockEditorComponents} from "./blockEditorComponents";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) await cleanup();
  vi.unstubAllGlobals();
});

async function render(children: React.ReactNode) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  await act(async () => root.render(children));
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  return host;
}

it("preserves the native table handle ref, rotation and drag callbacks", async () => {
  const Handle = blockEditorComponents.TableHandle.Root;
  const ref = createRef<HTMLButtonElement>(), start = vi.fn(), end = vi.fn();
  const host = await render(<Handle ref={ref} className="bn-table-handle" draggable label="列操作"
    onDragStart={start} onDragEnd={end} style={{transform: "rotate(0.25turn)"}}><span>upstream artwork</span></Handle>);
  const button = host.querySelector("button")!;
  expect(ref.current).toBe(button);
  expect(button.getAttribute("draggable")).toBe("true");
  expect(button.style.transform).toBe("rotate(0.25turn)");
  expect(button.getAttribute("aria-label")).toBe("列操作");
  expect(button.querySelectorAll(".note-table-handle-dots circle")).toHaveLength(6);
  expect(button.textContent).not.toContain("upstream artwork");
  await act(async () => {
    button.dispatchEvent(new Event("dragstart", {bubbles: true}));
    button.dispatchEvent(new Event("dragend", {bubbles: true}));
  });
  expect(start).toHaveBeenCalledOnce(); expect(end).toHaveBeenCalledOnce();
});

it("retains the separate cell-menu artwork", async () => {
  const Handle = blockEditorComponents.TableHandle.Root;
  const host = await render(<Handle className="bn-table-cell-handle" draggable={false}
    onDragStart={() => undefined} onDragEnd={() => undefined}><span>cell menu icon</span></Handle>);
  expect(host.textContent).toContain("cell menu icon");
  expect(host.querySelector(".note-table-handle-dots")).toBeNull();
});

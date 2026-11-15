// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { NotesProvider } from "../../../core/features/notes/NotesClient";
import { HbsAdaptersPreview } from "./HbsAdaptersPreview";
import { noteRendererGateway } from "./noteGateway";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", { configurable: true, value: "CSS1Compat" });
});
let dispose: (() => Promise<void>) | undefined;
afterEach(async () => { await dispose?.(); vi.unstubAllGlobals(); });
async function frame() {
  await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
}
async function click(label: string) {
  const button = [...document.querySelectorAll<HTMLElement>('button,[role="menuitem"]')]
    .find(element => element.textContent === label || element.getAttribute("aria-label") === label);
  if (!button) throw new Error(`missing ${label}`);
  await act(async () => button.click()); await frame();
}

it("previews native note editing and shared global review controls through the actual adapter page", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  dispose = async () => { await act(async () => root.unmount()); host.remove(); };
  await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}><HbsAdaptersPreview/></NotesProvider>));
  await frame();
  const editor = host.querySelector(".hbs-editor-frame .note-editor-surface .bn-editor");
  expect(editor).not.toBeNull();
  const controls = () => host.querySelector(".hbs-editor-frame .note-editor-surface .note-editor-diff-controls");
  expect(controls()).toBeNull();
  await click("修改 S：块内容");
  await click("段落");
  await click("同步 S → H");
  expect(host.querySelector(".bn-editor")).toBe(editor);
  expect(controls()?.textContent).toContain("1/1");
  expect(controls()?.textContent).toContain("Keep all");
  expect(host.querySelector(".note-diff-chunk-actions")).not.toBeNull();
  await click("Keep all");
  expect(controls()).toBeNull();
  expect(host.querySelector(".note-diff-chunk-actions")).toBeNull();
  expect(host.querySelector(".hbs-editor-frame > header")?.textContent).toContain("0 处待审 · 0 处冲突");
  expect(host.querySelector(".bn-editor")).toBe(editor);
});

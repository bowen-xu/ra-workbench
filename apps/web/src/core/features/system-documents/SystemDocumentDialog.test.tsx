/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { createMockSystemDocuments } from "../../../app/runtime/mock/mockSystemDocuments";
import { OPEN_TEXT_DOCUMENT, type TextDocumentRequest } from "../../components/text-editor/textDocumentEvents";
import { SystemDocumentDialog } from "./SystemDocumentDialog";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

async function harness(writeFailure = false) {
  localStorage.clear();
  const { gateway } = createMockSystemDocuments(localStorage);
  const write = vi.fn(writeFailure ? async () => { throw new Error("文档已变化"); } : gateway.write);
  const read = vi.fn(gateway.read), close = vi.fn();
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  let request: TextDocumentRequest | undefined;
  const listener = (event: Event) => { request = (event as CustomEvent<TextDocumentRequest>).detail; event.preventDefault(); };
  window.addEventListener(OPEN_TEXT_DOCUMENT, listener);
  await act(async () => root.render(<SystemDocumentDialog id="structured-task" gateway={{ ...gateway, read, write }} onClose={close} />));
  return { gateway, write, read, close, get request() { return request; },
    async open() { await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="编辑 阅读任务提示词.md"]')!.click()); },
    async dispose() { window.removeEventListener(OPEN_TEXT_DOCUMENT, listener); await act(async () => root.unmount()); host.remove(); },
  };
}
it("lists documents without rendering or reading their bodies and opens an editable stable tab", async () => {
  const h = await harness();
  try {
    expect(document.querySelectorAll(".system-document-row").length).toBeGreaterThan(1);
    expect(document.querySelector(".system-document-dialog .cm-editor")).toBeNull();
    expect(h.read).not.toHaveBeenCalled();
    const label = document.querySelector<HTMLElement>(".system-document-copy")!;
    expect(label.tagName).toBe("DIV");
    await act(async () => label.click());
    expect(h.request).toBeUndefined();
    expect(h.close).not.toHaveBeenCalled();
    expect(h.read).not.toHaveBeenCalled();
    await h.open();
    expect(h.request?.id).toBe("system:structured-task");
    expect(h.request?.readOnly).not.toBe(true);
    expect(h.close).toHaveBeenCalledOnce();
    const before = await h.gateway.read("structured-task");
    await h.request!.save!(before.content + "\n用户新规则。", before.baseline);
    const saved = await h.gateway.read(before.id);
    expect(saved.content).toContain("用户新规则。");
    await h.request!.save!(saved.content, saved.content);
    expect((await h.gateway.read(before.id)).baseline).toBe(saved.content);
    expect(h.write.mock.calls[1]![0].expectedRevision).toBe(saved.revision);
  } finally { await h.dispose(); }
});
it("propagates revision conflicts to the text session without overwriting the source", async () => {
  const h = await harness(true);
  try {
    await h.open();
    await expect(h.request!.save!("changed", "before")).rejects.toThrow("文档已变化");
    expect((await h.gateway.read("structured-task")).content).not.toBe("changed");
  } finally { await h.dispose(); }
});

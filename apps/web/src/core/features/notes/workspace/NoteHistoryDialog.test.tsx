// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { notePageId, noteRevisionId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { NoteHistoryDialog } from "./NoteHistoryDialog";
vi.mock("../editor/NoteSnapshotView", () => ({ NoteSnapshotView: () => <div>只读历史正文</div> }));
let host: HTMLDivElement, root: ReturnType<typeof createRoot>;
beforeEach(() => { vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
const pageId = notePageId("history-page"), id = noteRevisionId("revision");
const revision = { id, pageId, pageRevision: 2, reason: "manual" as const, createdAt: "2026-10-21T01:00:00Z", isPinned: false };
const snapshot: NoteDocumentV1 = { schemaVersion: 1, pageId, blocks: [] };
it("previews without writing and requires an explicit restore after loading", async () => {
  let deliver!: (document: NoteDocumentV1) => void;
  const load = vi.fn(() => new Promise<NoteDocumentV1>(resolve => { deliver = resolve; })), restore = vi.fn(async () => {}), close = vi.fn();
  await act(async () => root.render(<NoteHistoryDialog revisions={[revision]} onLoad={load} onRestore={restore} onClose={close} onPin={() => {}} readOnly={false} />));
  const buttons = () => [...document.querySelectorAll<HTMLButtonElement>('button')];
  const submit = () => buttons().find(button => button.textContent === "恢复此版本")!;
  expect(submit().disabled).toBe(true);
  await act(async () => buttons().find(button => button.textContent?.startsWith("版本 2"))!.click());
  expect(load).toHaveBeenCalledWith(id); expect(restore).not.toHaveBeenCalled(); expect(submit().disabled).toBe(true);
  await act(async () => deliver(snapshot));
  expect(document.body.textContent).toContain("只读历史正文"); expect(restore).not.toHaveBeenCalled();
  await act(async () => submit().click());
  expect(restore).toHaveBeenCalledWith(id); expect(close).toHaveBeenCalledOnce();
});
it("keeps conflict/lifecycle history available as a readonly preview", async () => {
  const restore = vi.fn(async () => {});
  await act(async () => root.render(<NoteHistoryDialog revisions={[revision]} onLoad={async () => snapshot} onRestore={restore} onClose={() => {}} onPin={() => {}} readOnly />));
  await act(async () => [...document.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent?.startsWith("版本 2"))!.click());
  expect([...document.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === "恢复此版本")!.disabled).toBe(true);
  expect(restore).not.toHaveBeenCalled();
});

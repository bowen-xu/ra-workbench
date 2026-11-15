// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { HbsSession, createNoteDocumentAdapter } from "@research-assistant/core/hbs";
import type { NoteAsset } from "@research-assistant/data/notes";
import { NotesProvider } from "../../features/notes/NotesClient";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import { noteFixture } from "../../../app/previews/hbs-adapters/fixtures";
import { noteRendererGateway } from "../../../app/previews/hbs-adapters/noteGateway";
import { directTextDiff } from "./codeMirrorHbsPolicy";
import { HbsNoteEditor } from "./HbsNoteEditor";

vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn(), info: vi.fn(), success: vi.fn() } }));
vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", { configurable: true, value: "CSS1Compat" });
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => { for (const cleanup of cleanups.splice(0)) await cleanup(); vi.unstubAllGlobals(); });
async function setup() {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  let finish: ((asset: NoteAsset) => void) | undefined;
  const deferred = new Promise<NoteAsset>(resolve => { finish = resolve; });
  const importing = vi.fn(() => deferred), status = vi.fn();
  const gateway = new Proxy(noteRendererGateway, { get(target, key) {
    if (key === "importNoteAsset") return importing;
    if (key === "readNoteAsset") return async () => Uint8Array.from([1, 2, 3]);
    return Reflect.get(target, key);
  } });
  const session = await HbsSession.open({ adapter: createNoteDocumentAdapter(directTextDiff), requestId: n => `asset-life-${n}`,
    preservation: { load: () => undefined, save: () => {} },
    port: { read: async () => ({ version: 1, saved: noteFixture, baseline: noteFixture }),
      commit: async request => ({ version: 2, saved: request.saved, baseline: request.baseline }) } });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let editor: NoteEditorInstance | undefined, mounted = true;
  const render = async (readOnly = false) => {
    await act(async () => root.render(<NotesProvider gateway={gateway}>
      <HbsNoteEditor session={session} isReadOnly={readOnly} onAssetStatus={status} onEditorReady={value => { editor = value; }}/>
    </NotesProvider>));
    await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
  };
  const unmount = async () => { if (mounted) { await act(async () => root.unmount()); mounted = false; } };
  cleanups.push(async () => { await unmount(); session.dispose(); host.remove(); });
  await render();
  return { session, render, unmount, importing, status,
    get editor() { if (!editor) throw new Error("missing editor"); return editor; },
    start: async () => {
      const input = host.querySelector<HTMLInputElement>('input[type="file"]')!;
      const file = new File(["fixture"], "fixture.txt", { type: "text/plain" });
      Object.defineProperty(file, "arrayBuffer", { value: async () => Uint8Array.from([1, 2, 3]).buffer });
      Object.defineProperty(input, "files", { configurable: true, value: [file] });
      await act(async () => { input.dispatchEvent(new Event("change", { bubbles: true })); });
      expect(importing).toHaveBeenCalledOnce();
    },
    finish: async () => { await act(async () => { finish?.({ id: "asset", displayName: "fixture.txt", mimeType: "text/plain", byteSize: 3 }); }); },
  };
}

it.each(["readonly", "unmounted"] as const)("does not insert a completed attachment after the note becomes %s", async state => {
  const f = await setup(); await f.start();
  const insert = vi.spyOn(f.editor, "insertBlocks"), before = f.session.getSnapshot().content;
  if (state === "readonly") await f.render(true); else await f.unmount();
  await f.finish();
  expect(insert).not.toHaveBeenCalled();
  expect(f.session.getSnapshot().content).toEqual(before);
  if (state === "unmounted") expect(f.status).not.toHaveBeenCalled();
});

it("inserts the imported attachment normally while the same active note remains editable", async () => {
  const f = await setup(); await f.start(); await f.finish();
  expect(f.session.getSnapshot().content.blocks.some(block => block.type === "file" && block.content.kind === "asset" && block.content.assetId === "asset")).toBe(true);
  expect(f.status).toHaveBeenCalledWith("已导入附件：fixture.txt");
});

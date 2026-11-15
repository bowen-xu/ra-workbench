// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { closeHistory, redo, undo } from "@tiptap/pm/history";
import { afterEach, expect, it, vi } from "vitest";
import { HbsSession, createNoteDocumentAdapter, DocumentVersionConflict,
  type DocumentCheckpoint, type DocumentCommit, type DocumentSnapshot } from "@research-assistant/core/hbs";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import { NotesProvider } from "../../features/notes/NotesClient";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import { noteFixture } from "../../../app/previews/hbs-adapters/fixtures";
import { noteRendererGateway } from "../../../app/previews/hbs-adapters/noteGateway";
import { directTextDiff } from "./codeMirrorHbsPolicy";
import { HbsNoteEditor } from "./HbsNoteEditor";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", { configurable: true, value: "CSS1Compat" });
});
const disposals: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const dispose of disposals.splice(0)) await dispose();
  vi.unstubAllGlobals();
});
async function settle() {
  await act(async () => { for (let index = 0; index < 80; index++) await Promise.resolve(); });
  await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
}
const path = (id = "hbs-score") => ["blocks", { key: "id", value: id }, "content", "items", 0, "text"] as const;

async function setup() {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const adapter = createNoteDocumentAdapter(directTextDiff);
  let authority: DocumentSnapshot<NoteDocumentV1> = { version: 23, saved: noteFixture, baseline: noteFixture };
  let preserved: DocumentCheckpoint<NoteDocumentV1> | undefined;
  let held = false;
  const releases: (() => void)[] = [], requests: DocumentCommit<NoteDocumentV1>[] = [];
  const session = await HbsSession.open({ adapter, requestId: sequence => `history-queue-${sequence}`,
    preservation: { load: () => preserved, save: value => { preserved = value; } },
    port: { read: async () => authority, commit: async request => {
      requests.push(request);
      if (held) await new Promise<void>(resolve => releases.push(resolve));
      if (request.expectedVersion !== authority.version) throw new DocumentVersionConflict(authority, request.id);
      authority = { version: authority.version + 1, saved: request.saved, baseline: request.baseline };
      return authority;
    } } });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  let editor: NoteEditorInstance | undefined;
  await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
    <HbsNoteEditor session={session} onEditorReady={value => { editor = value; }}/>
  </NotesProvider>));
  await settle();
  disposals.push(async () => { await act(async () => root.unmount()); session.dispose(); host.remove(); });
  const native = () => { if (!editor) throw new Error("missing native editor"); return editor; };
  return { adapter, session, host, requests,
    get nativeView() { return native().prosemirrorView; },
    get saved() { return authority; },
    state: () => session.getSnapshot(),
    value: (document: NoteDocumentV1, id = "hbs-score") => adapter.read(document, path(id)),
    external: async (text: string, synchronize = true, id = "hbs-score") => {
      authority = { ...authority, version: authority.version + 1, saved: adapter.replace(authority.saved, path(id), text) };
      if (synchronize) await act(async () => session.syncLatest());
      await settle();
    },
    input: async (from: number, to: number, text: string, id = "hbs-score") => {
      await act(async () => {
        const current = native(); current.setTextCursorPosition(id, "start");
        const start = current.prosemirrorState.selection.from;
        // Each replacement corresponds to a separate native event in the
        // supplied trace, including the deletion before inserting "8".
        current.prosemirrorView.dispatch(closeHistory(current.prosemirrorState.tr.insertText(text, start + from, start + to)));
      });
      await settle();
    },
    replay: async (direction: "before" | "after", count = 1) => {
      for (let index = 0; index < count; index++) {
        await act(async () => {
          const current = native();
          expect((direction === "before" ? undo : redo)(current.prosemirrorState, transaction => current.prosemirrorView.dispatch(transaction))).toBe(true);
        });
        await settle();
      }
    },
    click: async (label: string, id?: string) => {
      const scope = id ? host.querySelector(`[data-id="${id}"]`) : host;
      const button = [...(scope?.querySelectorAll("button") ?? [])].find(button => button.textContent === label || button.getAttribute("aria-label") === label);
      if (!button) throw new Error(`missing ${label} in ${id ?? "editor"}: ${JSON.stringify({
        labels: [...host.querySelectorAll("button")].map(button => button.getAttribute("aria-label") ?? button.textContent),
        score: adapter.read(session.getSnapshot().content, path()), baseline: adapter.read(session.getSnapshot().baselineProjection, path()),
        conflicts: session.getSnapshot().conflicts, queued: session.getSnapshot().queued, pending: session.getSnapshot().pending,
      })}`);
      await act(async () => button.click()); await settle();
    },
    save: async () => { await act(async () => session.save()); await settle(); },
    hold: () => { held = true; },
    release: async () => {
      held = false; await act(async () => releases.splice(0).forEach(resolve => resolve())); await settle();
    },
  };
}

it("finishes the reported native history trace without reviving already reviewed same-range compensation", async () => {
  const f = await setup();
  const nativeView = f.nativeView;
  await f.external("外部修改 v24 · paragraph");
  expect(f.nativeView).toBe(nativeView);
  await f.click("保留第 1 处变更");
  expect(f.saved.version).toBe(25);
  await f.input(6, 7, "1");
  expect(f.nativeView).toBe(nativeView);
  expect(f.value(f.state().baselineProjection)).toBe("外部修改 v14 · paragraph");
  await f.external("外部修改 v26 · paragraph");
  expect(f.nativeView).toBe(nativeView);
  expect(f.saved.version).toBe(27);
  await f.input(6, 7, "");
  await f.input(6, 6, "8");
  expect(f.nativeView).toBe(nativeView);
  expect(f.value(f.state().content)).toBe("外部修改 v86 · paragraph");
  await f.external("外部修改 v28 · paragraph", false);
  await f.save();
  expect(f.nativeView).toBe(nativeView);
  expect(f.state().conflicts).toHaveLength(1);
  await f.replay("before", 2); await f.replay("after", 2);
  expect(f.nativeView).toBe(nativeView);
  await f.click("Undo");
  expect(f.value(f.saved.saved)).toBe("外部修改 v14 · paragraph");
  await f.replay("before");
  await f.click("Keep");
  expect(f.value(f.saved.baseline)).toBe("外部修改 v86 · paragraph");
  await f.replay("before");
  await f.replay("before", 4); await f.replay("after", 5);
  expect(f.nativeView).toBe(nativeView);
  // The fifth Redo is the explicitly accepted Keep. With obsolete opposite
  // compensation removed, it has completed rather than leaving a phantom
  // conflict button. Only a further native Undo may reopen that decision.
  expect(f.state().conflicts).toHaveLength(0);
  expect(f.state().queued).toBe(0);
  expect(f.value(f.state().content)).toBe("外部修改 v86 · paragraph");
  expect(f.value(f.saved.baseline)).toBe("外部修改 v86 · paragraph");
  await f.replay("before");
  const displayedOld = f.value(f.state().baselineProjection);
  await f.click("Undo");
  expect(f.state().conflicts).toHaveLength(0);
  expect(f.state().queued).toBe(0);
  expect(f.state().pending).toBe(false);
  expect(f.value(f.state().content)).toBe(displayedOld);
  expect(f.value(f.saved.saved)).toBe(displayedOld);
  expect(f.value(f.saved.baseline)).toBe(displayedOld);
  expect(f.host.querySelector('[aria-label="incoming"]')).toBeNull();
  expect(f.nativeView).toBe(nativeView);
  const version = f.saved.version;
  await f.save();
  expect(f.saved.version).toBe(version);
  expect(f.state().queued).toBe(0);
  expect(f.value(f.saved.saved)).toBe(displayedOld);
  expect(f.value(f.saved.baseline)).toBe(displayedOld);
  const undoneVersion = f.saved.version;
  await f.replay("before");
  expect(f.saved.version).toBeGreaterThan(undoneVersion);
  expect(f.value(f.state().content)).toBe("外部修改 v86 · paragraph");
  expect(f.state().conflicts).toHaveLength(1);
  const replayVersion = f.saved.version;
  await f.replay("after");
  expect(f.saved.version).toBeGreaterThan(replayVersion);
  expect(f.state().conflicts).toHaveLength(0);
  expect(f.state().queued).toBe(0);
  expect(f.value(f.state().content)).toBe(displayedOld);
  expect(f.value(f.saved.saved)).toBe(displayedOld);
  expect(f.value(f.saved.baseline)).toBe(displayedOld);
  expect(f.nativeView).toBe(nativeView);
});

it("resolves a queued same-block Undo through the real controls while retaining another block's queued Undo", async () => {
  const f = await setup();
  await f.external("模型准确率为 85%。");
  await f.external("下一步：外部实验。", true, "hbs-next");
  await f.input(7, 9, "91");
  f.hold();
  await act(async () => { void f.session.save(); }); await settle();
  expect(f.state().pending).toBe(true);
  await f.click("撤销第 1 处变更");
  await f.click("撤销第 1 处变更");
  await f.input(7, 9, "92");
  await f.release();
  expect(f.state().conflicts.some(conflict => conflict.origin === "review")).toBe(true);
  const displayedOld = f.value(f.state().baselineProjection);
  // Typing after the earlier Undo has ended that candidate is ordinary input;
  // the red old includes it, and the later Undo must retain that same value.
  expect(displayedOld).toBe("模型准确率为 92%。");
  await f.click("Undo");
  await f.save();
  expect(f.state().conflicts).toHaveLength(0);
  expect(f.state().queued).toBe(0);
  expect(f.value(f.saved.saved)).toBe(displayedOld);
  expect(f.value(f.saved.saved, "hbs-next")).toBe("下一步：补充复杂场景的验证。");
  expect(f.saved.baseline).toEqual(f.adapter.replace(noteFixture, path(), displayedOld));
});

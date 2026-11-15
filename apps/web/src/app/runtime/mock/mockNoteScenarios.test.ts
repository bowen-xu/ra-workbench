// @vitest-environment happy-dom
import { beforeEach, expect, it, vi } from "vitest";
import { noteBlockId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { selectAppStorage } from "../../../core/runtime/appStorage";
import { createPreviewControls } from "../../../core/runtime/previewControls";
import { browserNotesGateway } from "../../../core/features/notes/gateways/browserNotesGateway";
import { NoteSaveController } from "../../../core/features/notes/editor/noteSaveController";
import { registerTextInstallPort } from "../../../core/features/notes/editor/noteTextEditorPorts";
import { MockNoteTextSession } from "./MockNoteTextSession";
import { createMockNoteScenarios } from "./mockNoteScenarios";
import { createMockTextPatchGateway, TEXT_PATCH_PREVIEW_PAGE } from "./mockTextPatchGateway";
import { readState, writeState } from "../../../core/features/notes/gateways/browserNotesStorage";
beforeEach(() => { localStorage.clear(); selectAppStorage(localStorage); });
async function fixture() {
  const created = await browserNotesGateway.createPage({ title: "Scenario fixture" });
  const state = await readState(), page = { ...created.page, id: TEXT_PATCH_PREVIEW_PAGE };
  const document: NoteDocumentV1 = { ...created.document, pageId: page.id, blocks: [{ id: noteBlockId("argument"), type: "paragraph", props: {}, content: { kind: "inline", items: [{ type: "text", text: "Original", styles: [] }] }, children: [] }] };
  await writeState({ ...state, pages: [page], documents: { [page.id]: document } });
  return { page, document };
}
it("injects a competing write through the actual CAS queue, retaining both snapshots", async () => {
  let current = await fixture(), live = current.document;
  const statuses: string[] = [];
  const controller = new NoteSaveController({ pageId: current.page.id, gateway: browserNotesGateway, initialRevision: 0,
    onSnapshot: value => { current = value; }, onStatus: value => statuses.push(value) });
  const session = new MockNoteTextSession(browserNotesGateway, TEXT_PATCH_PREVIEW_PAGE);
  const release = session.register({ controller, gateway: browserNotesGateway, getSnapshot: () => current, onError: () => {}, onChanged: () => {} });
  const unregister = registerTextInstallPort(TEXT_PATCH_PREVIEW_PAGE, {
    get isInstalling() { return false; }, read: () => live, prepare: (_before, after) => () => { live = after; return undefined; }, prepareStructure: () => () => undefined, dispose: () => {},
  });
  const changed = (value: string) => ({ ...live, blocks: live.blocks.map(b => ({ ...b, content: { kind: "inline" as const, items: [{ type: "text" as const, text: value, styles: [] }] } })) });
  try {
    await session.conflict(() => ({ current: changed("Current"), saved: changed("Competing") }));
    expect(statuses.at(-1)).toBe("conflict"); expect(controller.hasUnsavedChanges()).toBe(true);
    expect(live.blocks[0]!.content).toMatchObject({ items: [{ text: "Current" }] });
    expect((await browserNotesGateway.loadDocument(TEXT_PATCH_PREVIEW_PAGE)).document.blocks[0]!.content).toMatchObject({ items: [{ text: "Competing" }] });
  } finally { unregister(); release(); controller.dispose(); }
});
it("can lose a committed copy receipt and replay it without making a second copy", async () => {
  const source = await fixture(), controls = createPreviewControls(localStorage, () => {});
  controls.configure({ delayMs: 0, nextFailure: null });
  const scenarios = createMockNoteScenarios(new MockNoteTextSession(browserNotesGateway, TEXT_PATCH_PREVIEW_PAGE), createMockTextPatchGateway(), controls);
  const request = { requestId: crypto.randomUUID(), document: source.document, expectedRevision: 0, copy: true };
  const apply = vi.fn(() => browserNotesGateway.resolveConflict(request));
  await scenarios.scenarios.run("resolution-unknown");
  await expect(scenarios.resolve(apply)).rejects.toMatchObject({ code: "operation_unknown" });
  expect((await readState()).pages).toHaveLength(2);
  const receipt = await scenarios.resolve(apply);
  expect(receipt.copy).toBeDefined(); expect((await readState()).pages).toHaveLength(2);
  await scenarios.scenarios.run("resolution-failure");
  await expect(scenarios.resolve(apply)).rejects.toMatchObject({ code: "storage_error" });
  expect(apply).toHaveBeenCalledTimes(2);
});

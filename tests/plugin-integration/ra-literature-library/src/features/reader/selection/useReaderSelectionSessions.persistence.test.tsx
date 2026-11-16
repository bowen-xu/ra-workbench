// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { noteBlockId, type ReadingNoteBlock } from "@research-assistant/core/notes";
import type { NotesGateway } from "@research-assistant/data/notes";
import { positionNoteSelection } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/positionNotes";
import { AnnotationController } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/annotations/AnnotationController";
import { useReaderSelectionSessions, type ReaderSelectionSessions } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/selection/useReaderSelectionSessions";
import { PdfViewController } from "../../../../../../../apps/web/src/core/components/pdf-view/PdfViewController";

const rect = { x: .1, y: .2, width: .3, height: .04 };
const note: ReadingNoteBlock = {
  id: noteBlockId("highlight"), noteType: "highlight", body: "", color: "#f4cf65", status: "active", isFavorite: false,
  revision: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now",
  source: { anchorId: "anchor", paperId: "paper", paperTitle: "Paper", attachmentId: "pdf", contentSha256: "a".repeat(64), pageIndex: 0,
    rects: [rect], quoteExact: "Source", quotePrefix: "", quoteSuffix: "", state: "exact" },
};
type Options = Parameters<typeof useReaderSelectionSessions>[0];
let root: ReturnType<typeof createRoot>, container: HTMLDivElement, viewer: HTMLDivElement, owner: AnnotationController;
let sessions: ReaderSelectionSessions;
let controller:Options["controller"];
const create = vi.fn(), update = vi.fn();
function Harness({ existingAnchor, transferredComment }: { existingAnchor: boolean; transferredComment?: string }) {
  sessions = useReaderSelectionSessions({
    active: true, viewerReady: true, characterSelectionAvailable: true, pageLabels: ["1"],
    // These ports exercise selection persistence only; unrelated reader and
    // knowledge commands are not part of this fixture.
    item: { id: "paper", title: "Paper" } as Options["item"],
    attachment: { id: "pdf", contentSha256: note.source.contentSha256 } as Options["attachment"],
    controller,
    annotationController: owner, knowledgeGateway: {} as Options["knowledgeGateway"], referenceGateway: {} as Options["referenceGateway"],
    viewerContainerRef: { current: viewer }, setAnnouncement: vi.fn(),
    initialSessions: [{ id: "selection", active: transferredComment === undefined, dirty: transferredComment !== undefined,
      ...(transferredComment === undefined ? {} : {actionDraft:{mode:"annotation" as const,
        note:"",referenceRawText:"Source",conceptDirty:false,referenceDirty:false}}),
      ...(existingAnchor ? { anchorId: "anchor" } : {}), pending: {
      text: "Source", rects: [rect], prefix: "", suffix: "", pageIndex: 0, pageLabel: "1", popupPageIndex: 0, popupRect: rect,
      segments: [{ pageIndex: 0, pageLabel: "1", rects: [rect], text: "Source", prefix: "", suffix: "" }],
    } }],
  });
  return null;
}
beforeEach(async () => {
  vi.resetAllMocks(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  controller={setStatus:vi.fn(),viewController:new PdfViewController()} as unknown as Options["controller"];
  create.mockImplementation(async draft => ({ ...note, color: draft.color ?? note.color }));
  update.mockImplementation(async patch => ({ ...note, color: patch.color, revision: 2 }));
  owner = new AnnotationController({ gateway: {
    listReadingNotes: async (query: { status?: string }) => query.status === "active" ? [note] : [], createReadingNote: create, updateReadingNote: update,
  } as unknown as NotesGateway });
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  viewer = document.createElement("div"); document.body.append(viewer);
});
afterEach(async () => { await act(async () => root.unmount()); owner.dispose(); container.remove(); viewer.remove(); vi.unstubAllGlobals(); });
it("keeps a clean selection on outside clicks and dismisses it only inside its PDF viewport", async () => {
  await act(async () => root.render(<Harness existingAnchor={false} />));
  await act(async () => document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true })));
  expect(sessions.activeSession?.id).toBe("selection");
  await act(async () => viewer.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 2 })));
  expect(sessions.activeSession?.id).toBe("selection");
  await act(async () => viewer.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0 })));
  expect(sessions.activeSession).toBeUndefined();
});
it("restores a folded draft through the shared selection store and clears it when discarded",async()=>{
  await act(async()=>root.render(<Harness existingAnchor={false}/>));
  expect(controller.viewController.getSnapshot().selection?.text).toBe("Source");
  await act(async()=>sessions.updateDraftState("selection",{mode:"concept",note:"",referenceRawText:"Source",conceptDirty:true,referenceDirty:false},true));
  await act(async()=>sessions.close("selection"));expect(controller.viewController.getSnapshot().selection).toBeUndefined();
  await act(async()=>sessions.activateDraft("selection"));expect(controller.viewController.getSnapshot().selection?.text).toBe("Source");
  await act(async()=>sessions.discard("selection"));expect(controller.viewController.getSnapshot().selection).toBeUndefined();
});
it("does not let an older folded form erase a newer ordinary selection",async()=>{
  await act(async()=>root.render(<Harness existingAnchor={false}/>));
  await act(async()=>sessions.updateDraftState("selection",{mode:"concept",note:"",referenceRawText:"Source",conceptDirty:true,referenceDirty:false},true));
  await act(async()=>sessions.startSelection());
  const next={text:"New",segments:[{pageIndex:0,text:"New",rects:[rect],prefix:"",suffix:""}],focusPageIndex:0,focusRect:rect};
  await act(async()=>sessions.completeSelection(next));
  expect(controller.viewController.getSnapshot().selection).toBe(next);
  await act(async()=>sessions.discard("selection"));expect(controller.viewController.getSnapshot().selection).toBe(next);
});
it("does not restore annotation comment sessions as reader drafts", async () => {
  await act(async () => root.render(<Harness existingAnchor={false} transferredComment="" />));
  expect(sessions.sessions).toEqual([]);expect(sessions.hasDirtyDrafts()).toBe(false);
});
it("keeps an edited form on PDF clicks and stages it as a draft on explicit close", async () => {
  await act(async () => root.render(<Harness existingAnchor={false} />));
  const draft = { mode: "concept" as const, note: "", referenceRawText: "", conceptDirty: true, referenceDirty: false };
  await act(async () => sessions.updateDraftState("selection", draft, true));
  await act(async () => viewer.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0 })));
  expect(sessions.activeSession?.id).toBe("selection");
  await act(async () => sessions.close("selection"));
  expect(sessions.activeSession).toBeUndefined();
  expect(sessions.drafts[0]?.actionDraft).toEqual(draft);
});
it("creates a highlight with the chosen color and original source selector", async () => {
  await act(async () => root.render(<Harness existingAnchor={false} />));
  await act(async () => sessions.save(sessions.activeSession!, "", { color: "#91d5a3" }));
  expect(create).toHaveBeenCalledWith(expect.objectContaining({ color: "#91d5a3", noteType: "highlight", quoteExact: "Source", selector: expect.objectContaining({ schemaVersion: 2 }) }));
  expect(sessions.activeSession).toBeUndefined();
});
it("opens an existing highlight's note and changes its color without creating another document", async () => {
  await owner.load("paper", "pdf"); await act(async () => root.render(<Harness existingAnchor={false} />));
  await act(async () => sessions.openSavedHighlight(note, 0, rect));
  expect(sessions.activeSession).toMatchObject({ annotationId: note.id, anchorId: "anchor", highlightColor: note.color, pending: { text: "Source", popupRect: rect } });
  await act(async () => sessions.save(sessions.activeSession!, "", { color: "#91d5a3" }));
  expect(create).not.toHaveBeenCalled(); expect(update).toHaveBeenCalledWith({ id: note.id, color: "#91d5a3", expectedRevision: 1 });
  const opened = vi.fn(); window.addEventListener("research-assistant:open-reading-note", opened);
  try {
    await act(async () => sessions.openSavedHighlight(owner.getSnapshot().annotations[0]!, 0, rect));
    await act(async () => sessions.save(sessions.activeSession!, "", { openEditor: true }));
    expect(opened.mock.calls[0]?.[0].detail).toBe(note.id); expect(create).not.toHaveBeenCalled();
    expect(sessions.activeSession).toBeUndefined();
  } finally { window.removeEventListener("research-assistant:open-reading-note", opened); }
});
it("keeps a dirty highlight form when reopened and stages it when selecting another source", async () => {
  await act(async () => root.render(<Harness existingAnchor={false} />));
  await act(async () => sessions.openSavedHighlight(note, 0, rect));
  const draft = { mode: "concept" as const, note: "", referenceRawText: "", conceptDirty: true, referenceDirty: false };
  await act(async () => sessions.updateDraftState(sessions.activeSession!.id, draft, true));
  await act(async () => sessions.openSavedHighlight(note, 0, rect));
  expect(sessions.activeSession?.actionDraft).toEqual(draft);
  await act(async () => sessions.openSavedHighlight({ ...note, id: noteBlockId("another") }, 0, rect));
  expect(sessions.drafts[0]).toMatchObject({ annotationId: note.id, dirty: true, actionDraft: draft });
});
it("updates an existing anchor's highlight without creating another reading document", async () => {
  await owner.load("paper", "pdf"); await act(async () => root.render(<Harness existingAnchor />));
  await act(async () => sessions.save(sessions.activeSession!, "", { color: "#91bde8" }));
  expect(create).not.toHaveBeenCalled(); expect(update).toHaveBeenCalledWith({ id: note.id, color: "#91bde8", expectedRevision: 1 });
  expect(owner.getSnapshot().annotations[0]?.color).toBe("#91bde8"); expect(sessions.activeSession).toBeUndefined();
});
it("retains the selection after failed color persistence so retry reuses the same anchor", async () => {
  await owner.load("paper", "pdf"); await act(async () => root.render(<Harness existingAnchor />));
  update.mockRejectedValueOnce(new Error("Cannot save color"));
  await act(async () => { await expect(sessions.save(sessions.activeSession!, "", { color: "#efa4bf" })).rejects.toThrow("Cannot save color"); });
  expect(sessions.activeSession?.anchorId).toBe("anchor"); expect(create).not.toHaveBeenCalled();
  await act(async () => sessions.save(sessions.activeSession!, "", { color: "#efa4bf" }));
  expect(update).toHaveBeenCalledTimes(2); expect(sessions.activeSession).toBeUndefined();
});

it("creates an unquoted position annotation and waits for a later double click to open its document", async () => {
  await act(async () => root.render(<Harness existingAnchor={false} />));
  const position = { x: .25, y: .7, width: .00001, height: .00001 };
  await act(async () => sessions.completeSelection(positionNoteSelection(0, position)));
  const opened = vi.fn(); window.addEventListener("research-assistant:open-reading-note", opened);
  try {
    await act(async () => sessions.save(sessions.activeSession!, "", { openEditor: false, color: "#91bde8" }));
    expect(create).toHaveBeenCalledWith(expect.objectContaining({ noteType: "annotation", color: "#91bde8", quoteExact: "", rects: [position],
      selector: expect.objectContaining({ segments: [expect.objectContaining({ exactText: "", rects: [position] })] }) }));
    expect(opened).not.toHaveBeenCalled(); expect(sessions.activeSession).toBeUndefined();
  } finally { window.removeEventListener("research-assistant:open-reading-note", opened); }
});

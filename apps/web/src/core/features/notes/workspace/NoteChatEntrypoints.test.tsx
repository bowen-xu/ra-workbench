/** @vitest-environment happy-dom */
import { act, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, afterEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type ReadingNoteBlock } from "@research-assistant/core/notes";
import { NoteDocumentView } from "./NoteDocumentView";
import { ReadingNoteSourceSection } from "../reading-notes/ReadingNoteSourceSection";

// A host must not recreate a private chat runtime when rendering a note.
vi.mock("../agent/NoteAgentPanel", () => ({
  NoteAgentPanel: () => { throw new Error("Legacy note chat mounted"); },
  ReadingNoteAgentPanel: () => { throw new Error("Legacy reading-note chat mounted"); },
}));
vi.mock("../editor/NoteEditor", () => ({ NoteEditor: () => <div role="textbox" aria-label="笔记正文" /> }));
vi.mock("../../../../../../../plugins/literature_library/src/features/knowledge/ReadingNoteResearchActions", () => ({ ReadingNoteResearchActions: () => null }));

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const container = document.createElement("div");
const root = createRoot(container);
afterEach(async () => { await act(async () => root.render(null)); });
afterAll(async () => { await act(async () => root.unmount()); });

it("renders the standalone note editor and history without its private chat entry", async () => {
  const id = notePageId("b1111111-1111-4111-8111-111111111111");
  const noop = () => undefined;
  const props: ComponentProps<typeof NoteDocumentView> = {
    snapshot: {
      page: { id, title: "研究笔记", position: 0, status: "active", isFavorite: false, revision: 1, schemaVersion: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" },
      document: { schemaVersion: 1, pageId: id, blocks: [] },
    },
    editorSessionId: 1, pages: [], readingNotes: [], researchTargets: [], saveStatus: "saved", saveMessage: undefined, outgoingLinks: [], backlinks: [], showHistory: true, revisions: [],
    onOpenPage: noop, onOpenReading: noop, onDocumentChange: noop, onInstallPort: noop,
    onAssetStatus: noop, onUpdatePage: noop, onOpenHistory: noop,
    onCloseHistory: noop, onDuplicate: noop, onArchive: noop, onTrash: noop, onRetrySave: noop,
    onLoadRevision: async () => ({ schemaVersion: 1, pageId: id, blocks: [] }), onRestoreRevision: async () => {}, onToggleRevisionPinned: noop,
  };
  await act(async () => root.render(<NoteDocumentView {...props} />));
  expect(container.querySelector('[aria-label="笔记正文"]')).not.toBeNull();
  expect(document.querySelector('[aria-label="版本历史"]')).not.toBeNull();
  expect(container.textContent).not.toContain("AI 笔记助手");
});

it("keeps reading-note editing and source navigation without a second chat window", async () => {
  const note: ReadingNoteBlock = {
    id: noteBlockId("b1111111-1111-4111-8111-111111111111"), noteType: "annotation", body: "已保存批注", color: "#ffcc00",
    status: "active", isFavorite: false, revision: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now",
    source: { paperId: "paper", paperTitle: "测试文献", attachmentId: "pdf", contentSha256: "a".repeat(64), pageIndex: 0,
      rects: [], quoteExact: "原文摘录", quotePrefix: "", quoteSuffix: "", state: "unavailable" },
  };
  await act(async () => root.render(<ReadingNoteSourceSection note={note} />));
  expect(container.querySelector("textarea")).toBeNull();
  expect(container.querySelector('[aria-label="返回原文"]')).not.toBeNull();
  expect(container.textContent).toContain("原文摘录");
  expect(container.textContent).not.toContain("AI 阅读笔记助手");
});

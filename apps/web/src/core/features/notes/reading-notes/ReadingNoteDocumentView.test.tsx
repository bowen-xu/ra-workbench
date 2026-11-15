// @vitest-environment happy-dom
import { act, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type ReadingNoteBlock } from "@research-assistant/core/notes";
import { NoteDocumentView } from "../workspace/NoteDocumentView";

const renderedEditor = vi.hoisted(() => ({props: undefined as import("../editor/NoteEditor").NoteEditorProps | undefined}));
vi.mock("../editor/NoteEditor", () => ({ NoteEditor: (props: import("../editor/NoteEditor").NoteEditorProps) => {
  renderedEditor.props = props; return <textarea aria-label="共享正文编辑器" />;
} }));
vi.mock("../../../../../../../plugins/literature_library/src/features/knowledge/ReadingNoteResearchActions", () => ({ ReadingNoteResearchActions: () => null }));
let container: HTMLDivElement, root: ReturnType<typeof createRoot>;
const id = notePageId("reading-page");
const note: ReadingNoteBlock = {
  id: noteBlockId("reading-source"), pageId: id, noteType: "annotation", body: "", color: "#ffcc00", status: "active",
  isFavorite: false, revision: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now",
  source: { paperId: "paper", paperTitle: "A paper title which must not become an editable note title", attachmentId: "pdf", contentSha256: "a".repeat(64), pageIndex: 2,
    rects: [], quoteExact: "Readonly source quotation", quotePrefix: "", quoteSuffix: "", state: "exact" },
};
function props(): ComponentProps<typeof NoteDocumentView> {
  const noop = () => undefined;
  return {
    readingSource: note, snapshot: { page: { id, title: "Internal page label", position: 0, status: "active", isFavorite: false, revision: 1, schemaVersion: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" }, document: { schemaVersion: 1, pageId: id, blocks: [] } },
    editorSessionId: 1, pages: [], readingNotes: [], researchTargets: [], saveStatus: "saved", saveMessage: undefined, outgoingLinks: [], backlinks: [], showHistory: false, revisions: [],
    onOpenPage: noop, onOpenReading: noop, onDocumentChange: noop, onInstallPort: noop,  onAssetStatus: noop,
    onUpdatePage: noop, onOpenHistory: noop, onCloseHistory: noop, onDuplicate: noop, onArchive: noop, onTrash: noop,
    onRetrySave: noop, onLoadRevision: async () => ({ schemaVersion: 1, pageId: id, blocks: [] }), onRestoreRevision: async () => {}, onToggleRevisionPinned: noop,
  };
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
it("renders a body-only reading surface without title, page properties or relations", async () => {
  await act(async () => root.render(<NoteDocumentView {...props()} />));
  expect(container.querySelector('[aria-label="共享正文编辑器"]')).not.toBeNull();
  expect(container.querySelector('[aria-label="笔记标题"]')).toBeNull();
  expect(container.querySelector(".note-page-breadcrumbs, .note-properties-panel, .note-relations")).toBeNull();
  expect(container.textContent).not.toContain("Internal page label");
  expect(container.querySelector("details")?.open).toBe(false);
  expect(container.querySelector("summary")?.textContent).toBe("来源 · 第 3 页");
  expect(container.querySelector("blockquote")?.textContent).toBe(note.source.quoteExact);
});
it("preserves input, selection and expanded source when history and save state update", async () => {
  const initial = props(); await act(async () => root.render(<NoteDocumentView {...initial} />));
  const editor = container.querySelector("textarea")!;
  editor.value = "Later input"; editor.setSelectionRange(2, 5);
  container.querySelector("details")!.open = true;
  await act(async () => root.render(<NoteDocumentView {...initial} saveStatus="saving" showHistory />));
  expect(container.querySelector("textarea")).toBe(editor); expect(editor.value).toBe("Later input"); expect(editor.selectionStart).toBe(2);
  expect(container.querySelector("details")!.open).toBe(true);
  expect(document.querySelector('[aria-label="版本历史"]')).not.toBeNull();
});
it("shares passive conflict status and preserves failure retry", async () => {
  const retry = vi.fn();
  const initial = { ...props(), onRetrySave: retry };
  await act(async () => root.render(<NoteDocumentView {...initial} saveStatus="failed" />));
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="重试保存或核对原操作"]')!.click());
  expect(retry).toHaveBeenCalledOnce();
  await act(async () => root.render(<NoteDocumentView {...initial} saveStatus="conflict" />));
  const actions = container.querySelector('.reading-note-document-actions')!;
  const status = actions.querySelector<HTMLElement>('[role="status"]')!;
  expect(status.querySelector('svg.lucide-circle-alert')).not.toBeNull();
  expect(actions.querySelector('[aria-label="解决冲突"]')).toBeNull();
  const saveDuringConflict = actions.querySelector<HTMLButtonElement>('[aria-label="保存并保全当前输入"]')!;
  expect(saveDuringConflict.disabled).toBe(false);
  await act(async () => { saveDuringConflict.click(); status.click(); });
  expect(retry).toHaveBeenCalledTimes(2);
  expect(document.querySelector('[aria-label="解决笔记冲突"]')).toBeNull();
});
it("keeps unavailable source feedback visible in the collapsed summary", async () => {
  await act(async () => root.render(<NoteDocumentView {...props()} readingSource={{ ...note, source: { ...note.source, state: "unavailable" } }} />));
  expect(container.querySelector("details")?.open).toBe(false);
  expect(container.querySelector("summary")?.textContent).toContain("来源不可用");
  expect(container.querySelector<HTMLButtonElement>('[aria-label="返回原文"]')?.disabled).toBe(true);
});

it.each([
  ["saved", "已保存"], ["dirty", "等待保存"], ["saving", "正在保存…"], ["failed", "保存失败"],
] as const)("presents %s as an icon with an accessible hint instead of visible text", async (saveStatus, label) => {
  await act(async () => root.render(<NoteDocumentView {...props()} saveStatus={saveStatus} saveMessage="状态详情" />));
  const status = container.querySelector<HTMLElement>('.reading-note-document-actions [role="status"]')!;
  expect(status.getAttribute("aria-label")).toBe(`${label}：状态详情`);
  expect(status.tabIndex).toBe(0); expect(status.textContent).toBe("");
  expect(status.querySelector('svg[aria-hidden="true"]')).not.toBeNull();
});

it.each([false, true])("places trash lifecycle icons in the existing toolbar and keeps the document read-only (reading=%s)", async reading => {
  const initial = props(), restore = vi.fn(), permanentlyDelete = vi.fn(), update = vi.fn();
  const { readingSource, ...regular } = initial;
  const current = { ...(reading ? initial : regular), onRestore: restore, onPermanentlyDelete: permanentlyDelete, onUpdatePage: update,
    snapshot: { ...initial.snapshot, page: { ...initial.snapshot.page, status: "trashed" as const } } };
  await act(async () => root.render(<NoteDocumentView {...current} />));
  expect(container.querySelector('.note-lifecycle-actions')).toBeNull();
  const actions = container.querySelector(reading ? '.reading-note-document-actions' : '.note-page-toolbar')!;
  const buttons = [...actions.querySelectorAll<HTMLButtonElement>('button')];
  expect(buttons.slice(0, 2).map(button => button.getAttribute('aria-label'))).toEqual(["恢复笔记", "永久删除"]);
  expect(buttons.slice(0, 2).every(button => button.classList.contains('note-page-icon-button') && button.textContent === "")).toBe(true);
  const favorite = buttons.filter(button => button.getAttribute('aria-label')?.startsWith('收藏'));
  if (!reading) expect(favorite).toHaveLength(1);
  expect(favorite.every(button => button.disabled)).toBe(true);
  if (!reading) {
    expect(renderedEditor.props?.isReadOnly).toBe(true);
    expect(renderedEditor.props?.pageTitle?.value).toBe(initial.snapshot.page.title);
    expect(update).not.toHaveBeenCalled();
  }
  await act(async () => { buttons[0]!.click(); buttons[1]!.click(); });
  expect(restore).toHaveBeenCalledOnce(); expect(permanentlyDelete).toHaveBeenCalledOnce();
  const editor = container.querySelector('textarea');
  await act(async () => root.render(<NoteDocumentView {...current} snapshot={initial.snapshot} />));
  expect(container.querySelector('[aria-label="恢复笔记"], [aria-label="永久删除"]')).toBeNull();
  expect(container.querySelector('textarea')).toBe(editor);
  if (!reading) expect(renderedEditor.props?.isReadOnly).toBe(false);
});

it("does not expose permanent deletion for archived notes or run busy lifecycle actions", async () => {
  const initial = props(), restore = vi.fn();
  await act(async () => root.render(<NoteDocumentView {...initial} lifecycleBusy onRestore={restore} onPermanentlyDelete={vi.fn()}
    snapshot={{...initial.snapshot, page: {...initial.snapshot.page, status: "archived"}}} />));
  const button = container.querySelector<HTMLButtonElement>('[aria-label="恢复笔记"]')!;
  expect(button.disabled).toBe(true);
  expect(container.querySelector('[aria-label="永久删除"]')).toBeNull();
  await act(async () => button.click()); expect(restore).not.toHaveBeenCalled();
});

it("allows restoring an inactive note before resolving its content conflict", async () => {
  const initial = props(), restore = vi.fn();
  await act(async () => root.render(<NoteDocumentView {...initial} saveStatus="conflict" onRestore={restore}
    snapshot={{...initial.snapshot, page: {...initial.snapshot.page, status: "trashed"}}} />));
  const button = container.querySelector<HTMLButtonElement>('[aria-label="恢复笔记"]')!;
  expect(button.disabled).toBe(false);
  await act(async () => button.click()); expect(restore).toHaveBeenCalledOnce();
  expect(container.querySelector('[aria-label="笔记标题"]')).toBeNull();
});

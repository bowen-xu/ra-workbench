// @vitest-environment happy-dom
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { notePageId } from "@research-assistant/core/notes";
import type { LiteratureItem } from "@research-assistant/core/literature";
import type { LiteratureNoteSummary, NotesGateway } from "@research-assistant/data/notes";
import { selectAppStorage } from "../../../../../../apps/web/src/core/runtime/appStorage";
import { LiteratureNotes } from "../../../../../../../plugins/ra-literature-library/src/features/reader/LiteratureNotes";
const state = vi.hoisted(() => ({ trash: vi.fn(), list: vi.fn(), create: vi.fn(), error: vi.fn(), notes: [] as LiteratureNoteSummary[] }));
vi.mock("../../../../../../apps/web/src/core/features/notes/reading-notes/literatureNotePanelActions", () => ({ requestLiteratureNoteTrash: state.trash }));
vi.mock("../../../../../../apps/web/src/core/features/notes/NotesClient", () => {
  const gateway = { listLiteratureNotes: state.list, createLiteratureNote: state.create };
  const client = { gateway, mutate: (operation: (gateway: NotesGateway) => Promise<unknown>) => operation(gateway as unknown as NotesGateway) };
  return { useNotesClient: () => client, useNotesVersion: () => 0 };
});
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { error: state.error } }));
let host: HTMLDivElement, root: ReturnType<typeof createRoot>;
const item = { id: "paper", title: "Synthetic paper", attachments: [] } as unknown as LiteratureItem;
function note(id: string): LiteratureNoteSummary {
  return { page: { id: notePageId(id), title: id, position: 0, status: "active", isFavorite: false, revision: 0, schemaVersion: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" }, preview: `Body ${id}` };
}
const ids = () => [...host.querySelectorAll<HTMLElement>("li[data-literature-note]")].map(row => row.dataset.literatureNote);
async function mount(): Promise<void> { await act(async () => root.render(<StrictMode><LiteratureNotes item={item} active /></StrictMode>)); }
beforeEach(() => {
  vi.clearAllMocks(); state.notes = [note("new"), note("old")]; state.list.mockImplementation(async () => state.notes);
  const data = new Map<string, string>(); selectAppStorage({ get length() { return data.size; }, key: index => [...data.keys()][index] ?? null,
    getItem: key => data.get(key) ?? null, setItem: (key, value) => { data.set(key, value); }, removeItem: key => { data.delete(key); }, clear: () => data.clear() });
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
it("creates one item note without page arguments, places it first and opens the shared editor", async () => {
  state.create.mockImplementation(async () => { const created = note("newest"); state.notes = [created, ...state.notes]; return { page: created.page }; });
  const opened = vi.fn(); window.addEventListener("research-assistant:open-literature-note", opened);
  try {
    await mount();
    const create = host.querySelector<HTMLButtonElement>(".reader-notes-heading .literature-note-create");
    expect(create?.getAttribute("aria-label")).toBe("写文献笔记");
    expect(create?.textContent).toBe("");
    expect(create?.hasAttribute("title")).toBe(false);
    expect(create?.querySelector(".lucide-square-pen")).not.toBeNull();
    await act(async () => create?.focus());
    await vi.waitFor(() => expect(document.querySelector('[role="tooltip"]')?.textContent).toContain("写文献笔记"));
    await act(async () => create?.click());
    expect(state.create).toHaveBeenCalledExactlyOnceWith("paper"); expect(ids()).toEqual(["newest", "new", "old"]);
    expect((opened.mock.calls[0]?.[0] as CustomEvent).detail).toBe("newest");
    state.create.mockRejectedValueOnce(new Error("write failure"));
    await act(async () => host.querySelector<HTMLButtonElement>(".literature-note-create")!.click());
    expect(state.error).toHaveBeenCalledWith("write failure"); expect(ids()).toEqual(["newest", "new", "old"]);
  } finally { window.removeEventListener("research-assistant:open-literature-note", opened); }
});
it("supports drag, cancels without reordering, and retains keyboard order after remount", async () => {
  await mount();
  for (const [index, row] of [...host.querySelectorAll<HTMLElement>("li")].entries()) row.getBoundingClientRect = () => new DOMRect(0, index * 50, 200, 40);
  const handle = host.querySelector<HTMLButtonElement>('[aria-label="调整文献笔记顺序：old"]')!;
  await act(async () => handle.dispatchEvent(new PointerEvent("pointerdown", { isPrimary: true, bubbles: true, button: 0, pointerId: 1, clientY: 70 })));
  await act(async () => window.dispatchEvent(new PointerEvent("pointercancel", { pointerId: 1, clientY: 10 })));
  expect(ids()).toEqual(["new", "old"]);
  await act(async () => handle.dispatchEvent(new PointerEvent("pointerdown", { isPrimary: true, bubbles: true, button: 0, pointerId: 2, clientY: 70 })));
  await act(async () => window.dispatchEvent(new PointerEvent("pointermove", { pointerId: 2, clientY: 10 })));
  expect(host.querySelector('li[data-literature-note="new"]')?.getAttribute("style")).toContain("--reorder-y: 50px");
  await act(async () => window.dispatchEvent(new PointerEvent("pointerup", { pointerId: 2, clientY: 10 })));
  expect(ids()).toEqual(["old", "new"]);
  await act(async () => handle.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" })));
  expect(ids()).toEqual(["new", "old"]);
  await act(async () => root.unmount()); root = createRoot(host); await mount(); expect(ids()).toEqual(["new", "old"]);
});
it("trashes a row through the editing host and preserves it on failure", async () => {
  await mount();
  state.trash.mockRejectedValueOnce(new Error("unsaved draft"));
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="删除文献笔记：old"]')!.click());
  expect(state.error).toHaveBeenCalledWith("unsaved draft"); expect(ids()).toEqual(["new", "old"]);
  state.trash.mockImplementation(async (id: string) => { state.notes = state.notes.filter(note => note.page.id !== id); });
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="删除文献笔记：old"]')!.click());
  expect(state.trash).toHaveBeenLastCalledWith("old"); expect(ids()).toEqual(["new"]);
  expect(host.querySelector(".reorderable-list-row .reorderable-drag-handle")).not.toBeNull();
});

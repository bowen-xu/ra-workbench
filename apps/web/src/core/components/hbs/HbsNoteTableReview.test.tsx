// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { compareTableRows } from "@research-assistant/core/hbs";
import { NotesProvider } from "../../features/notes/NotesClient";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import { noteRendererGateway } from "../../../app/previews/hbs-adapters/noteGateway";
import { createTableScenario, tableRows } from "../../../app/previews/hbs-table-review/scenarios";
import { HbsNoteEditor } from "./HbsNoteEditor";
import type { NoteTableComparison } from "./noteTableDecorations";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", { configurable: true, value: "CSS1Compat" });
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => { for (const cleanup of cleanups.splice(0)) await cleanup(); vi.unstubAllGlobals(); });
async function setup(mode: Parameters<typeof createTableScenario>[0] = "combined", tableComparison?: NoteTableComparison) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const scenario = await createTableScenario(mode);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); let editor: NoteEditorInstance | undefined;
  cleanups.push(async () => { await act(async () => root.unmount()); scenario.session.dispose(); host.remove(); });
  await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
    <HbsNoteEditor session={scenario.session} {...(tableComparison ? { tableComparison } : {})} onEditorReady={value => { editor = value; }}/>
  </NotesProvider>));
  await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
  const table = () => host.querySelector<HTMLTableElement>('.bn-editor [data-id="table-review-grid"] [data-content-type="table"] table')!;
  return { ...scenario, host, table, get editor() { if (!editor) throw new Error("editor not mounted"); return editor; },
    click: async (label: string) => {
      const button = host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)
        ?? Array.from(host.querySelectorAll("button")).find(button => button.textContent === label);
      if (!button) throw new Error(`missing ${label}`);
      await act(async () => button.click());
    } };
}
it("renders the inserted row and changed original cell in one real table, with one whole-table action", async () => {
  const f = await setup();
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.host.querySelectorAll(".bn-editor table")).toHaveLength(1);
  expect(f.table().rows).toHaveLength(4);
  expect(f.table().rows[1]!.classList.contains("hbs-table-row-added")).toBe(true);
  const row = f.table().rows[2]!;
  expect(row.cells[0]!.textContent).toBe("无噪声");
  expect(row.cells[0]!.querySelector(".note-diff-deleted-text")).toBeNull();
  expect(row.cells[1]!.querySelector(".note-diff-deleted-tableCell")?.textContent).toBe("0.80");
  expect(row.cells[1]!.querySelector(".hbs-table-cell-current")?.textContent).toBe("0.44");
  expect(f.table().rows[3]!.querySelector(".note-diff-added-text,.note-diff-deleted-text")).toBeNull();
  expect(f.host.querySelectorAll('button[aria-label="保留第 1 处变更"]')).toHaveLength(1);
  expect(f.host.querySelector('button[aria-label="保留第 2 处变更"]')).toBeNull();
});

it("allows hosts to replace the shared default table correspondence policy", async () => {
  const comparison = vi.fn(compareTableRows);
  const f = await setup("combined", comparison);
  expect(comparison).toHaveBeenCalled();
  expect(f.host.querySelectorAll(".bn-editor table")).toHaveLength(1);
  expect(f.table().querySelector(".hbs-table-cell-current")?.textContent).toBe("0.44");
});

it.each(["combined", "plain"] as const)("keeps rendering through repeated external row insertions after mounting: %s", async mode => {
  const f = await setup(mode);
  if (mode === "plain") await act(async () => f.editCell());
  const initialRows = f.table().rows.length, insertedRows = f.table().querySelectorAll(".hbs-table-row-added").length;
  for (let step = 0; step < 3; step++) {
    await act(async () => f.insertRow());
    expect(f.table().rows).toHaveLength(initialRows + 1 + step);
    expect(f.table().querySelectorAll(".hbs-table-row-added")).toHaveLength(insertedRows + 1 + step);
    expect(f.table().querySelector(".hbs-table-cell-current")?.textContent).toBe("0.44");
    expect(f.host.querySelectorAll('button[aria-label="保留第 1 处变更"]')).toHaveLength(1);
  }
});
it("shows removed rows as legal readonly rows without adding them to the editable document", async () => {
  const f = await setup(); await act(async () => f.deleteRow());
  const removed = f.table().querySelector<HTMLTableRowElement>("tr.hbs-table-row-removed")!;
  expect(removed).not.toBeNull(); expect(removed.getAttribute("contenteditable")).toBe("false");
  expect(removed.cells).toHaveLength(2); expect(removed.cells[0]!.textContent).toBe("有噪声");
  expect(removed.querySelector("table")).toBeNull();
  expect(tableRows(f.session.getSnapshot().content)).toHaveLength(3);
  expect(f.table().rows).toHaveLength(4);
});
it("keeps current cells editable and reviews the entire table through native undo and redo", async () => {
  const f = await setup(); let from: number | undefined;
  f.editor.prosemirrorState.doc.descendants((node, position) => { if (node.type.name === "tableParagraph" && node.textContent === "0.44") from = position + 1; });
  if (from === undefined) throw new Error("current cell missing");
  const position = from;
  await act(async () => f.editor.prosemirrorView.dispatch(f.editor.prosemirrorState.tr.insertText("0.43", position, position + 4)));
  expect(f.table().querySelector(".hbs-table-cell-current")?.textContent).toBe("0.43");
  await f.click("保留第 1 处变更");
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.table().querySelector(".hbs-table-row-added,.note-diff-deleted-tableCell")).toBeNull();
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.table().querySelector(".hbs-table-row-added")).not.toBeNull();
  await act(async () => { expect(f.editor.redo()).toBe(true); });
  expect(f.session.getSnapshot().changes).toHaveLength(0);
});
it("uses daff to show an added identical row in one editable table", async () => {
  const f = await setup("duplicates");
  expect(f.table().rows).toHaveLength(4);
  expect(f.table().querySelectorAll(".hbs-table-row-added")).toHaveLength(1);
  expect(f.table().rows[3]!.classList.contains("hbs-table-row-added")).toBe(true);
  expect(f.host.querySelector(".note-diff-deleted-blocks")).toBeNull();
  expect(f.host.querySelector(".hbs-table-unmatched,.hbs-table-comparison-reason")).toBeNull();
  expect(f.host.querySelector(".hbs-table-row-removed")).toBeNull();
  expect(f.host.querySelectorAll(".bn-editor table")).toHaveLength(1);
  await f.click("保留第 1 处变更");
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.host.querySelector(".note-diff-deleted-blocks,.note-diff-added-block")).toBeNull();
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  expect(f.table().querySelectorAll(".hbs-table-row-added")).toHaveLength(1);
});

it.each([0, 1, 2])("keeps original cells aligned and editable across an inserted column at %s", async column => {
  const f = await setup(); await act(async () => f.insertColumn(column));
  expect(f.host.querySelectorAll(".bn-editor table")).toHaveLength(1);
  expect(f.table().rows).toHaveLength(4);
  for (const row of Array.from(f.table().rows)) {
    expect(row.cells).toHaveLength(3);
    expect(row.cells[column]!.classList.contains("hbs-table-column-added")).toBe(true);
  }
  const accuracy = f.table().rows[2]!.cells[column <= 1 ? 2 : 1]!;
  expect(accuracy.querySelector(".note-diff-deleted-tableCell")?.textContent).toBe("0.80");
  expect(accuracy.querySelector(".hbs-table-cell-current")?.textContent).toBe("0.44");
  await f.click("保留第 1 处变更");
  expect(f.table().querySelector(".hbs-table-column-added")).toBeNull();
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  expect(f.table().querySelectorAll(".hbs-table-column-added")).toHaveLength(4);
});

it("keeps cell-only actions inside the changed cell with the comparison configured", async () => {
  const f = await setup("plain"); await act(async () => f.editCell());
  const cell = f.table().rows[1]!.cells[1]!;
  expect(cell.querySelector(".hbs-note-cell-actions")).not.toBeNull();
  expect(cell.querySelector(".hbs-note-cell-current")?.textContent).toBe("0.44");
  expect(f.host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(1);
  await f.click("撤销第 1 处变更");
  expect(f.session.getSnapshot().changes).toHaveLength(0);
});

it("keeps a rectangular live table through repeated columns and later row edits", async () => {
  const f = await setup("plain");
  await act(async () => f.editCell());
  await act(async () => f.insertColumn());
  await act(async () => f.insertColumn());
  await act(async () => f.insertRow());
  await act(async () => f.deleteRow());
  for (const row of Array.from(f.table().rows)) expect(row.cells).toHaveLength(4);
  expect(f.table().querySelector(".hbs-table-cell-current")?.textContent).toBe("0.44");
  expect(f.table().querySelector(".hbs-table-row-removed")?.textContent).toContain("有噪声");
  expect(f.host.querySelectorAll('button[aria-label^="保留第"]')).toHaveLength(1);
});

it.each([false, true])("retains human new across column insertion then shows actual incoming, saved: %s", async saved => {
  const f = await setup("plain"); await act(async () => f.editCell());
  let from: number | undefined;
  f.editor.prosemirrorState.doc.descendants((node, position) => { if (node.type.name === "tableParagraph" && node.textContent === "0.44") from = position + 1; });
  if (from === undefined) throw new Error("current cell missing");
  const position = from;
  await act(async () => f.editor.prosemirrorView.dispatch(f.editor.prosemirrorState.tr.insertText("0.43", position, position + 4)));
  if (saved) await act(async () => f.session.save());
  await act(async () => f.insertColumn());
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  expect(f.table().rows[1]!.cells[2]!.textContent).toContain("0.43");
  expect(f.table().querySelectorAll(".hbs-table-column-added")).toHaveLength(3);
  await act(async () => f.editCell());
  expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
  expect(JSON.stringify(f.session.getSnapshot().content)).toContain("0.43");
  expect(f.host.querySelector('[aria-label="incoming"]')?.textContent).toContain("0.45");
  expect(f.host.querySelector('[aria-label="incoming"]')?.getAttribute("contenteditable")).toBe("false");
  await f.click("Take");
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  expect(f.session.getSnapshot().changes.length).toBeGreaterThan(0);
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
  await f.click("Both");
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  expect(JSON.stringify(f.session.getSnapshot().content)).toContain("0.45");
  expect(JSON.stringify(f.session.getSnapshot().content)).toContain("0.43");
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
  await f.click(saved ? "Keep" : "Undo");
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
  await act(async () => { expect(f.editor.redo()).toBe(true); });
  expect(f.session.getSnapshot().conflicts, JSON.stringify({ conflicts: f.session.getSnapshot().conflicts, events: f.session.getSnapshot().events })).toHaveLength(0);
});

it("discards all grouped changes together even when insert/delete cancel the row count, and can undo that choice", async () => {
  const f = await setup(); await act(async () => f.deleteRow());
  expect(f.session.getSnapshot().changes.length).toBeGreaterThan(1);
  expect(f.host.querySelectorAll('button[aria-label^="撤销第"]')).toHaveLength(1);
  await f.click("撤销第 1 处变更");
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.table().rows).toHaveLength(3);
  expect(f.table().rows[1]!.cells[0]!.textContent).toBe("无噪声");
  expect(f.table().rows[2]!.cells[0]!.textContent).toBe("有噪声");
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  expect(f.table().querySelector(".hbs-table-row-added")).not.toBeNull();
  expect(f.table().querySelector(".hbs-table-row-removed")).not.toBeNull();
});

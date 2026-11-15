// @vitest-environment happy-dom
import {act, type ComponentProps} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {notePageId} from "@research-assistant/core/notes";
import {DeveloperModeProvider} from "../../features/settings/DeveloperMode";
import {NotePageHeader as ProductionNotePageHeader} from "../../features/notes/workspace/NotePageHeader";
import {ReadingNoteDocumentHeader as ProductionReadingNoteDocumentHeader} from "../../features/notes/reading-notes/ReadingNoteDocumentHeader";
import {NotesProvider} from "../../features/notes/NotesClient";
import {registerNoteOperationLogActions, type NoteOperationLogActions} from "./noteOperationLogActions";

function NotePageHeader(props:ComponentProps<typeof ProductionNotePageHeader>){return <NotesProvider><ProductionNotePageHeader {...props}/></NotesProvider>;}
function ReadingNoteDocumentHeader(props:ComponentProps<typeof ProductionReadingNoteDocumentHeader>){return <NotesProvider><ProductionReadingNoteDocumentHeader {...props}/></NotesProvider>;}

let container: HTMLDivElement, root: ReturnType<typeof createRoot>;
const cleanups: (() => void)[] = [];
const labels = ["复制操作记录", "下载操作记录 JSON", "清空操作记录"];
function commands(): NoteOperationLogActions {
  return {copy: vi.fn(async () => {}), download: vi.fn(async () => {}), clear: vi.fn(), copying: false, downloading: false};
}
function ordinaryProps(id = "page-a"): ComponentProps<typeof NotePageHeader> {
  const noop = () => undefined;
  return {page: {id: notePageId(id), title: "Test note", position: 0, status: "active", isFavorite: false, revision: 1,
    schemaVersion: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now"},
    pages: [], saveStatus: "saved", saveMessage: undefined,
    onOpenPage: noop, onFavoriteChange: noop,  onOpenRelations: noop, onOpenHistory: noop, onDuplicate: noop, onArchive: noop, onTrash: noop, onRetrySave: noop};
}
async function openSubmenu(kind: "ordinary" | "reading"): Promise<Element> {
  await act(async () => container.querySelector<HTMLButtonElement>(`[aria-label="${kind === "ordinary" ? "更多页面操作" : "笔记操作"}"]`)!.click());
  const trigger = document.querySelector<HTMLButtonElement>('[aria-label="笔记操作记录"]')!;
  expect(trigger.tagName).toBe("BUTTON");
  expect(trigger.closest('[role="menu"]')).not.toBeNull();
  expect(container.querySelector('.note-page-diff-controls')?.children).toHaveLength(0);
  await act(async () => trigger.click());
  return document.querySelector('[role="menu"][aria-label="笔记操作记录"]')!;
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.setItem("research-assistant.developer-mode", "true");
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => {
  await act(async () => {root.unmount(); cleanups.splice(0).forEach(cleanup => cleanup());});
  container.remove(); localStorage.removeItem("research-assistant.developer-mode"); vi.unstubAllGlobals();
});

it.each(["ordinary", "reading"] as const)("provides only three diagnostic commands in the production %s menu", async kind => {
  const actions = commands(); cleanups.push(registerNoteOperationLogActions("page-a", actions));
  await act(async () => root.render(<DeveloperModeProvider>{kind === "ordinary" ? <NotePageHeader {...ordinaryProps()}/> :
    <ReadingNoteDocumentHeader pageId="page-a" contextTitle="Reading context" saveStatus="saved" saveMessage={undefined}
      onOpenHistory={() => {}} onTrash={() => {}} onRetrySave={() => {}}/>}</DeveloperModeProvider>));
  for (const [index, label] of labels.entries()) {
    const submenu = await openSubmenu(kind);
    expect([...submenu.querySelectorAll('[role="menuitem"]')].map(item => item.textContent)).toEqual(labels);
    expect(submenu.closest('[data-overlay-portal="popup"]')).not.toBeNull();
    const button = [...submenu.querySelectorAll<HTMLButtonElement>('button')].find(item => item.textContent === label)!;
    await act(async () => button.click());
    expect([actions.copy, actions.download, actions.clear][index]).toHaveBeenCalledOnce();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  }
});

it("hides diagnostic commands outside developer mode even if a retained owner is registered", async () => {
  localStorage.setItem("research-assistant.developer-mode", "false");
  cleanups.push(registerNoteOperationLogActions("page-a", commands()));
  await act(async () => root.render(<DeveloperModeProvider><NotePageHeader {...ordinaryProps()}/></DeveloperModeProvider>));
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="更多页面操作"]')!.click());
  expect(document.querySelector('[aria-label="笔记操作记录"]')).toBeNull();
});

it("addresses the current page and disables only the command already in progress", async () => {
  const first = commands(), second = {...commands(), copying: true};
  cleanups.push(registerNoteOperationLogActions("page-a", first), registerNoteOperationLogActions("page-b", second));
  await act(async () => root.render(<DeveloperModeProvider><NotePageHeader {...ordinaryProps("page-b")}/></DeveloperModeProvider>));
  const submenu = await openSubmenu("ordinary");
  const buttons = [...submenu.querySelectorAll<HTMLButtonElement>('button')];
  expect(buttons.map(button => button.disabled)).toEqual([true, false, false]);
  await act(async () => buttons[2]!.click());
  expect(second.clear).toHaveBeenCalledOnce(); expect(first.clear).not.toHaveBeenCalled();
});

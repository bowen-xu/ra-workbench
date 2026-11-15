// @vitest-environment happy-dom
import { act, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { notePageId, type NotePage } from "@research-assistant/core/notes";
import { NotePageHeader as ProductionNotePageHeader } from "./NotePageHeader";
import {NotesProvider} from "../NotesClient";

function NotePageHeader(props:ComponentProps<typeof ProductionNotePageHeader>):React.JSX.Element {
  return <NotesProvider><ProductionNotePageHeader {...props}/></NotesProvider>;
}

let container: HTMLDivElement, root: ReturnType<typeof createRoot>;
function page(id: string, parentPageId?: NotePage["id"]): NotePage {
  return { id: notePageId(id), title: id, position: 0, status: "active", isFavorite: false, revision: 1,
    schemaVersion: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now",
    ...(parentPageId === undefined ? {} : { parentPageId }) };
}
function props(): ComponentProps<typeof NotePageHeader> {
  const noop = () => undefined;
  return { page: page("研究计划"), pages: [], saveStatus: "saved", saveMessage: undefined,
    onOpenPage: noop, onFavoriteChange: noop,
    onOpenRelations: noop, onOpenHistory: noop, onDuplicate: noop, onArchive: noop, onTrash: noop,
    onRetrySave: noop };
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });

it("omits the fixed workspace label and the empty path for a root page", async () => {
  await act(async () => root.render(<NotePageHeader {...props()} />));
  expect(container.querySelector('[aria-label="页面路径"]')).toBeNull();
  expect(container.textContent).not.toContain("笔记");
  expect(container.querySelector('[aria-label="笔记标题"]')).toBeNull();
});

it("omits legacy page properties and tag chips even for notes with stored tags",async()=>{
  await act(async()=>root.render(<NotePageHeader {...props()} page={{...page("历史笔记"),tagIds:["stored-tag"]}}/>));
  expect(container.querySelector('[aria-label="页面属性"], [aria-label="页面属性摘要"], [aria-label="页面标签"], [aria-label="移动页面"]')).toBeNull();
  expect(container.querySelector('.note-document-header, #note-page-properties')).toBeNull();
  expect(container.querySelector('[aria-label="收藏页面"]')).not.toBeNull();
  expect(container.querySelector('[aria-label="更多页面操作"]')).not.toBeNull();
});

it("uses the ordinary toolbar for an immediately created empty page",async()=>{
  await act(async()=>root.render(<NotePageHeader {...props()} page={{...page("empty"),title:"无标题"}}/>));
  expect(container.querySelector('[aria-label="收藏页面"]')).not.toBeNull();
  expect(container.querySelector('[aria-label="更多页面操作"]')).not.toBeNull();
  expect(container.textContent).not.toContain("临时草稿");
});

it("keeps ancestor navigation without a leading separator", async () => {
  const parent = page("父页面"), child = page("子页面", parent.id), leaf = page("当前页面", child.id);
  const open = vi.fn();
  await act(async () => root.render(<NotePageHeader {...props()} page={leaf} pages={[parent, child, leaf]} onOpenPage={open} />));
  const path = container.querySelector('[aria-label="页面路径"]')!;
  expect([...path.querySelectorAll("button")].map(button => button.textContent)).toEqual(["父页面", "子页面"]);
  expect(path.querySelectorAll("svg")).toHaveLength(1);
  await act(async () => path.querySelector("button")!.click());
  expect(open).toHaveBeenCalledWith(parent.id);
});

it.each([
  ["saved", "已保存"], ["dirty", "等待保存"], ["saving", "正在保存…"], ["failed", "保存失败"],
] as const)("keeps %s readable and keyboard accessible as an icon", async (saveStatus, label) => {
  await act(async () => root.render(<NotePageHeader {...props()} saveStatus={saveStatus} saveMessage="状态详情" />));
  const status = container.querySelector<HTMLElement>('[role="status"]')!;
  expect(status.getAttribute("aria-label")).toBe(`${label}：状态详情`);
  expect(status.tabIndex).toBe(0);
  expect(status.textContent).toBe("");
  expect(status.querySelector('svg[aria-hidden="true"]')).not.toBeNull();
});

it("removes page ordering commands while keeping history usable", async () => {
  const history = vi.fn();
  await act(async () => root.render(<NotePageHeader {...props()} onOpenHistory={history} />));
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="更多页面操作"]')!.click());
  const menu = document.querySelector('[role="menu"]')!;
  expect(menu.textContent).not.toMatch(/上移页面|下移页面/);
  expect([...menu.querySelectorAll('[role="menuitem"]')].map(item => item.textContent)).toEqual(["出链与反链", "版本历史", "导出", "创建副本", "归档", "移到回收站"]);
  await act(async () => [...menu.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "版本历史")!.click());
  expect(history).toHaveBeenCalledOnce();
});

it("keeps retry for failures and keeps normal conflict passive and preserves failure retry", async () => {
  const retry = vi.fn();
  await act(async () => root.render(<NotePageHeader {...props()} saveStatus="failed" onRetrySave={retry} />));
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="重试保存或核对原操作"]')!.click());
  expect(retry).toHaveBeenCalledOnce();
  expect(container.querySelector('[role="status"] svg.lucide-circle-alert')).not.toBeNull();
  await act(async () => root.render(<NotePageHeader {...props()} saveStatus="conflict" saveMessage="MODEL_TEST_CONFLICT: request-123" onRetrySave={retry} />));
  const toolbar = container.querySelector('[role="toolbar"]')!;
  const status = toolbar.querySelector<HTMLElement>('[role="status"]')!;
  expect(status.querySelector('svg.lucide-circle-alert')).not.toBeNull();
  expect(status.getAttribute('aria-label')).toContain('MODEL_TEST_CONFLICT: request-123');
  expect(status.getAttribute('aria-label')).toContain('Take／Both');
  expect(toolbar.querySelector('[aria-label="解决冲突"]')).toBeNull();
  const saveDuringConflict = toolbar.querySelector<HTMLButtonElement>('[aria-label="保存并保全当前输入"]')!;
  expect(saveDuringConflict.disabled).toBe(false);
  await act(async () => { saveDuringConflict.click(); status.click(); });
  expect(retry).toHaveBeenCalledTimes(2);
  expect(document.querySelector('[role="menu"][aria-label="笔记恢复与诊断"]')).toBeNull();
  expect(document.querySelector('[role="dialog"]')).toBeNull();
});

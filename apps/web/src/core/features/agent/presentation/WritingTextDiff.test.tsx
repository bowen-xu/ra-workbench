/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import { getChunks, getOriginalDoc } from "@codemirror/merge";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { WritingTextDiff } from "./WritingTextDiff";
let root:Root; let container:HTMLDivElement;
beforeEach(()=>{
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
  container=document.createElement("div");document.body.append(container);root=createRoot(container);
});
afterEach(async()=>{await act(async()=>root.unmount());container.remove();vi.restoreAllMocks();});
async function render(before:string,after:string):Promise<EditorView> {
  await act(async()=>root.render(<WritingTextDiff before={before} after={after}/>));
  const element=container.querySelector('.cm-editor');
  const view=element && EditorView.findFromDOM(element as HTMLElement);
  if(!view) throw new Error('Missing editor'); return view;
}
it("renders real changed chunks read-only with original Unicode, CRLF and trailing newline intact",async()=>{
  const before="Same\r\n旧🐶\r\nLast";const after="Same\r\n新🐯\r\nLast\n";
  const view=await render(before,after);
  expect(view.state.doc.toString()).toBe(after);expect(getOriginalDoc(view.state).toString()).toBe(before);
  expect(view.state.readOnly).toBe(true);expect(view.contentDOM.contentEditable).toBe("false");
  expect(getChunks(view.state)?.chunks).toHaveLength(1);
  const deletedChunk = container.querySelector('.cm-deletedChunk') as HTMLElement | null;
  expect(deletedChunk).not.toBeNull();
  expect(deletedChunk?.contentEditable).toBe("false");
  expect(deletedChunk?.querySelector("del")?.textContent).toContain("旧🐶");
  expect(container.querySelector('.cm-merge-revert')).toBeNull();
  expect(container.textContent).toContain("文件末尾换行：无 → 有");
  const button=[...container.querySelectorAll('button')].find((b)=>b.textContent==='下一处变更');
  await act(async()=>button?.click());
  expect(view.state.selection.main.head).toBeGreaterThan(0);
  expect(view.state.doc.toString()).toBe(after);
});
it("keeps the view on unchanged props and destroys it when the reviewed version changes",async()=>{
  const destroy=vi.spyOn(EditorView.prototype,'destroy');
  const first=await render("old","new");
  expect(await render("old","new")).toBe(first);expect(destroy).not.toHaveBeenCalled();
  const second=await render("previous","current");
  expect(second).not.toBe(first);expect(destroy).toHaveBeenCalledTimes(1);
  expect(first.dom.isConnected).toBe(false);expect(container.querySelectorAll('.cm-editor')).toHaveLength(1);
});
it("updates frozen after bytes in the read-only view without exposing editing or review commands", async () => {
  const current = await render("old\r\n", "new\r\n");
  expect(await render("old\r\n", "later\r\n")).toBe(current);
  expect(current.state.doc.toString()).toBe("later\r\n");
  expect(getOriginalDoc(current.state).toString()).toBe("old\r\n");
  expect(current.state.readOnly).toBe(true);
  expect(current.contentDOM.contentEditable).toBe("false");
  expect(container.querySelector(".writing-diff-control-row")).toBeNull();
  expect(container.querySelector(".writing-diff-chunk-actions")).toBeNull();
  expect(container.querySelector(".cm-merge-revert")).toBeNull();
});
it("navigates between diffs without wrapping at either boundary",async()=>{
  await act(async()=>root.render(<WritingTextDiff before={"a\nold\nb\nold2\nc"} after={"a\nnew\nb\nnew2\nc"} showSummary={false}/>));
  const element=container.querySelector('.cm-editor');
  const view=element && EditorView.findFromDOM(element as HTMLElement);
  if(!view) throw new Error('Missing editor');
  const chunks=getChunks(view.state)?.chunks ?? [];
  expect(chunks).toHaveLength(2);
  await act(async()=>view.dispatch({selection:{anchor:chunks[1]!.fromB}}));
  view.dispatch({selection:{anchor:0}});
  const next=[...container.querySelectorAll("button")].find(button => button.textContent === "下一处变更")!;
  const previous=[...container.querySelectorAll("button")].find(button => button.textContent === "上一处变更")!;
  await act(async()=>next.click());
  expect(view.state.selection.main.head).toBe(chunks[0]!.fromB);
  await act(async()=>next.click());
  expect(view.state.selection.main.head).toBe(chunks[1]!.fromB);
  await act(async()=>next.click());
  expect(view.state.selection.main.head).toBe(chunks[1]!.fromB);
  await act(async()=>previous.click());
  expect(view.state.selection.main.head).toBe(chunks[0]!.fromB);
  await act(async()=>previous.click());
  expect(view.state.selection.main.head).toBe(chunks[0]!.fromB);
});
it.each([["","added"],["deleted",""],["same","same"],["",""],["a\n","a"]])("handles empty and unchanged sides: %j → %j",async(before,after)=>{
  const view=await render(before,after);
  expect(view.state.doc.toString()).toBe(after);expect(getOriginalDoc(view.state).toString()).toBe(before);
  expect(getChunks(view.state)?.chunks.length===0).toBe(before===after);
});
it("keeps large content intact while rendering only the viewport",async()=>{
  const before="line\n".repeat(20000);const after=before+"追加";
  const view=await render(before,after);
  expect(view.state.doc.toString()).toBe(after);
  expect(container.querySelectorAll('.cm-line').length).toBeLessThan(20000);
});

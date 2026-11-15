// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {MenuProvider} from "@ariakit/react";
import {BlockNoteEditor} from "@blocknote/core";
import {BlockNoteViewRaw} from "@blocknote/react";
import {TableHandlesExtension} from "@blocknote/core/extensions";
import {afterEach,expect,it,vi} from "vitest";
import {NoteTableHandleMenu} from "./TableHandleMenu";
import {blockEditorDefaults} from "./defaults";
import {baseBlockEditorSchema} from "./baseSchema";

// Mock only hover location. The block, header command, transactions, history,
// live state subscriptions and shared overlay/menu components remain real.
const hover=vi.hoisted(()=>({rowIndex:0,colIndex:0}));
vi.mock("@blocknote/react",async original=>{
  const actual=await original<typeof import("@blocknote/react")>();
  return {...actual,useExtensionState:(extension:unknown,...args:unknown[])=>extension===TableHandlesExtension
    ? {block:{id:"table"},...hover} : Reflect.apply(actual.useExtensionState,undefined,[extension,...args])};
});
let cleanup:(()=>Promise<void>)|undefined;
afterEach(async()=>{await cleanup?.();cleanup=undefined;vi.unstubAllGlobals();});
async function mount(orientation:"row"|"column",index=0){
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);hover.rowIndex=index;hover.colIndex=index;
  const engine=BlockNoteEditor.create({...blockEditorDefaults,schema:baseBlockEditorSchema,initialContent:[{id:"table",type:"table",content:{type:"tableContent",rows:[{cells:["A","B"]},{cells:["C","D"]}]}}]});
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  await act(async()=>root.render(<BlockNoteViewRaw editor={engine} formattingToolbar={false} sideMenu={false} tableHandles={false} slashMenu={false}>
    <MenuProvider open><NoteTableHandleMenu orientation={orientation}/></MenuProvider>
  </BlockNoteViewRaw>));
  cleanup=async()=>{await act(async()=>root.unmount());host.remove();};return engine;
}
it.each(["row","column"] as const)("shows the first %s header switch above Delete and keeps native Undo",async orientation=>{
  const engine=await mount(orientation),key=orientation==="row" ? "headerRows" : "headerCols";
  const items=()=>[...document.querySelectorAll<HTMLElement>('.note-table-handle-menu [role^="menuitem"]')];
  expect(items()[0]?.textContent).toBe(orientation==="row" ? "表头行" : "表头列");
  expect(items()[1]?.textContent).toBe(orientation==="row" ? "删除行" : "删除列");
  const toggle=items()[0]!;expect(toggle.getAttribute("aria-checked")).toBe("false");
  expect(toggle.closest('[data-overlay-portal="popup"]')).not.toBeNull();
  await act(async()=>toggle.click());
  expect(toggle.getAttribute("aria-checked")).toBe("true");
  expect(toggle.isConnected).toBe(true);
  expect(engine.getBlock("table")?.content).toMatchObject({[key]:1});
  await act(async()=>{expect(engine.undo()).toBe(true);});
  expect(toggle.getAttribute("aria-checked")).toBe("false");
  await act(async()=>{expect(engine.redo()).toBe(true);});
  expect(toggle.getAttribute("aria-checked")).toBe("true");
  await act(async()=>toggle.click());expect(toggle.getAttribute("aria-checked")).toBe("false");
});
it.each(["row","column"] as const)("does not offer headers on a later %s",async orientation=>{
  await mount(orientation,1);
  expect(document.querySelector('.note-table-handle-menu [role="menuitemcheckbox"]')).toBeNull();
  expect(document.querySelector('.note-table-handle-menu')?.textContent).toContain(orientation==="row" ? "删除行" : "删除列");
});

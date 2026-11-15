// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { HbsSession, compareTableRows, createNoteDocumentAdapter, type DocumentSnapshot } from "@research-assistant/core/hbs";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import { NotesProvider } from "../../features/notes/NotesClient";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import { noteFixture } from "../../../app/previews/hbs-adapters/fixtures";
import { noteRendererGateway } from "../../../app/previews/hbs-adapters/noteGateway";
import { directTextDiff } from "./codeMirrorHbsPolicy";
import { HbsNoteEditor } from "./HbsNoteEditor";

vi.hoisted(()=>{
  if(!document.doctype)document.insertBefore(document.implementation.createDocumentType("html","",""),document.documentElement);
  Object.defineProperty(document,"compatMode",{configurable:true,value:"CSS1Compat"});
});
const cleanups:(()=>Promise<void>)[]=[];
afterEach(async()=>{for(const cleanup of cleanups.splice(0))await cleanup();vi.unstubAllGlobals();});
const path=["blocks",{key:"id",value:"hbs-table"},"content","rows",1,1] as const;
const inline=(text:string)=>({kind:"inline" as const,items:[{type:"text" as const,text,styles:[]}]});
async function setup(initial=noteFixture,compare=false){
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const adapter=createNoteDocumentAdapter(directTextDiff);
  let saved:DocumentSnapshot<NoteDocumentV1>={version:1,saved:initial,baseline:initial},editor:NoteEditorInstance|undefined;
  const session=await HbsSession.open({adapter,requestId:n=>`cell-${n}`,preservation:{load:()=>undefined,save:()=>{}},
    port:{read:async()=>saved,commit:async request=>{expect(request.expectedVersion).toBe(saved.version);saved={version:saved.version+1,saved:request.saved,baseline:request.baseline};return saved;}}});
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  cleanups.push(async()=>{await act(async()=>root.unmount());session.dispose();host.remove();});
  await act(async()=>root.render(<NotesProvider gateway={noteRendererGateway}><HbsNoteEditor session={session} {...(compare?{tableComparison:compareTableRows}:{})} onEditorReady={value=>{editor=value;}}/></NotesProvider>));
  await act(async()=>{await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));});
  const table=()=>host.querySelector<HTMLTableElement>('.bn-editor [data-content-type="table"] table')!;
  return {adapter,session,host,table,get editor(){if(!editor)throw new Error("editor not mounted");return editor;},get saved(){return saved;},
    external:async(document:NoteDocumentV1)=>{saved={...saved,version:saved.version+1,saved:document};await act(async()=>session.syncLatest());},
    click:async(label:string)=>{const button=[...host.querySelectorAll("button")].find(button=>button.textContent===label||button.getAttribute("aria-label")===label);
      if(!button)throw new Error(`missing ${label}`);await act(async()=>button.click());}};
}

it("reviews only cell text with actions after the green paragraph and preserves native history",async()=>{
  const f=await setup();await f.external(f.adapter.replace(noteFixture,path,inline("0.44")));
  const cell=f.table().rows[1]!.cells[1]!;
  expect(f.host.querySelectorAll('.bn-editor')).toHaveLength(1);
  expect(cell.classList.contains("note-diff-added-block")).toBe(false);
  expect(cell.querySelector("table")).toBeNull();
  expect(cell.querySelector('.note-diff-deleted-tableCell')?.textContent).toBe("0.80");
  const current=cell.querySelector('.hbs-note-cell-current')!;
  expect(current?.tagName).toBe("P");expect(current?.textContent).toBe("0.44");
  expect(current.querySelector('.note-diff-added-text')?.textContent).toContain("44");
  const actions=cell.querySelector('.hbs-note-cell-actions')!;
  expect(actions).not.toBeNull();expect(actions.closest('.note-diff-added-block')).toBeNull();
  expect(current.compareDocumentPosition(actions)&Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(f.table().rows[2]!.cells[1]!.querySelector('.note-diff-added-block,.note-diff-deleted-content')).toBeNull();
  await f.click("保留第 1 处变更");expect(f.adapter.read(f.saved.baseline,path)).toEqual(inline("0.44"));
  await act(async()=>{expect(f.editor.undo()).toBe(true);});
  expect(f.adapter.read(f.saved.baseline,path)).toEqual(f.adapter.read(noteFixture,path));
  expect(f.table().rows[1]!.cells[1]!.querySelector("table")).toBeNull();
  await act(async()=>{expect(f.editor.redo()).toBe(true);});
  expect(f.session.getSnapshot().changes).toHaveLength(0);
});

it.each([false,true])("keeps old/incoming/current inside one real cell with comparison configured: %s",async compare=>{
  const f=await setup(noteFixture,compare);await f.external(f.adapter.replace(noteFixture,path,inline("0.90")));
  let position:number|undefined;
  f.editor.prosemirrorState.doc.descendants((node,from)=>{if(node.type.name==="tableParagraph"&&node.textContent==="0.90")position=from;});
  if(position===undefined)throw new Error("live cell paragraph missing");
  const from=position+1;await act(async()=>f.editor.prosemirrorView.dispatch(f.editor.prosemirrorState.tr.insertText("0.91",from,from+4)));
  await f.external(f.adapter.replace(f.saved.saved,path,inline("0.92")));
  const cell=f.table().rows[1]!.cells[1]!;
  expect(cell.querySelectorAll("table")).toHaveLength(0);
  expect(cell.querySelector('[aria-label="incoming"]')?.textContent).toBe("0.92");
  expect(cell.querySelector('[aria-label="incoming"]')?.getAttribute("contenteditable")).toBe("false");
  expect(cell.querySelector('.hbs-note-cell-current')?.textContent).toBe("0.91");
  expect(cell.querySelector('.hbs-note-cell-actions')?.closest('.note-diff-added-block,.hbs-note-incoming')).toBeNull();
  await f.click("Take");expect(f.adapter.read(f.session.getSnapshot().content,path)).toEqual(inline("0.92"));
  await act(async()=>{expect(f.editor.undo()).toBe(true);});
  expect(f.table().rows[1]!.cells[1]!.querySelector('[aria-label="incoming"]')?.textContent).toBe("0.92");
});

it("preserves rich marks and atomic formula rendering in cell snapshots",async()=>{
  const adapter=createNoteDocumentAdapter(directTextDiff);
  const content=(source:string)=>({kind:"inline" as const,items:[{type:"text" as const,text:"formula ",styles:["bold" as const]},
    {type:"inlineFormula" as const,source}]});
  const initial=adapter.replace(noteFixture,path,content("x^2"));
  const f=await setup(initial);await f.external(f.adapter.replace(initial,path,content("y^2")));
  const cell=f.table().rows[1]!.cells[1]!;
  const old=cell.querySelector('.note-diff-deleted-tableCell');
  expect(old).not.toBeNull();
  expect(old?.querySelector('strong,b')).not.toBeNull();
  expect(old?.querySelector('.katex')).not.toBeNull();
  expect(cell.querySelector('.hbs-note-cell-current .katex')).not.toBeNull();
  expect(cell.querySelector("table")).toBeNull();
});

it("rejects one cell's text without accepting or discarding another cell", async () => {
  const f = await setup();
  const otherPath = ["blocks", { key: "id", value: "hbs-table" }, "content", "rows", 2, 1] as const;
  const changed = f.adapter.replace(f.adapter.replace(noteFixture, path, inline("0.44")), otherPath, inline("0.66"));
  await f.external(changed);
  expect(f.session.getSnapshot().changes).toHaveLength(2);

  await f.click("撤销第 1 处变更");

  expect(f.adapter.read(f.saved.saved, path)).toEqual(inline("0.80"));
  expect(f.adapter.read(f.saved.saved, otherPath)).toEqual(inline("0.66"));
  expect(f.adapter.read(f.saved.baseline, otherPath)).toEqual(inline("0.65"));
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.table().rows[1]!.cells[1]!.querySelector(".hbs-note-cell-actions")).toBeNull();
  expect(f.table().rows[2]!.cells[1]!.querySelector(".hbs-note-cell-current")?.textContent).toBe("0.66");
});

it("keeps an emptied cell editable with its actions below the empty paragraph", async () => {
  const f = await setup();
  await f.external(f.adapter.replace(noteFixture, path, inline("")));
  const cell = f.table().rows[1]!.cells[1]!;
  const current = cell.querySelector(".hbs-note-cell-current")!;
  const actions = cell.querySelector(".hbs-note-cell-actions")!;

  expect(current.tagName).toBe("P");
  expect(current.textContent).toBe("");
  expect(actions).not.toBeNull();
  expect(current.compareDocumentPosition(actions) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(actions.closest(".note-diff-added-block")).toBeNull();
  expect(cell.querySelector("table")).toBeNull();
});

it("keeps a single HBS cell edit local even without another column to anchor daff", async () => {
  const adapter = createNoteDocumentAdapter(directTextDiff);
  const rows = ["blocks", { key: "id", value: "hbs-table" }, "content", "rows"] as const;
  const initial = adapter.replace(noteFixture, rows, [[inline("header")], [inline("old")], [inline("stable")]]);
  const f = await setup(initial, true);
  await f.external(adapter.replace(initial, [...rows, 1, 0], inline("changed")));
  const cell = f.table().rows[1]!.cells[0]!;
  expect(cell.querySelector(".hbs-note-cell-current")?.textContent).toBe("changed");
  expect(cell.querySelector(".hbs-note-cell-actions")).not.toBeNull();
  expect(f.host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(1);
});

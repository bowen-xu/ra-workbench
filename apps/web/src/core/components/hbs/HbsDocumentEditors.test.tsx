import { HbsStorageRows } from "./HbsStorageRows";
// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { EditorView } from "@codemirror/view";
import { undo, redo } from "@codemirror/commands";
import { HbsSession, compareTableRows, createTextDocumentAdapter, createNoteDocumentAdapter, createStructuredDocumentAdapter, DocumentVersionConflict, type DocumentCheckpoint, type DocumentSnapshot, type HbsDocumentAdapter } from "@research-assistant/core/hbs";
import { HbsNoteEditor } from "./HbsNoteEditor";
import { HbsStructuredEditor } from "./HbsStructuredEditor";
import { directTextDiff } from "./codeMirrorHbsPolicy";
import { noteFixture, structuredFixture } from "../../../app/previews/hbs-adapters/fixtures";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import { NotesProvider } from "../../features/notes/NotesClient";
import { noteRendererGateway } from "../../../app/previews/hbs-adapters/noteGateway";
import { HbsSnapshotSource } from "./HbsSnapshotSource";
import { HbsLeafEditor } from "./HbsLeafEditor";
import { noteBlockId } from "@research-assistant/core/notes";
import { HbsClient } from "@research-assistant/core/hbs";
import { HbsWireAuthority, HbsWireClient, MemoryHbsRepository } from "@research-assistant/data/hbs";
import { HbsTextEditor } from "./HbsTextEditor";
import { createCodeMirrorHbsPolicy } from "./codeMirrorHbsPolicy";
import { holdTextEditor } from "../../features/notes/editor/noteTextEditorPorts";
vi.hoisted(()=>{
  if(document.doctype===null)document.insertBefore(document.implementation.createDocumentType("html","",""),document.documentElement);
  Object.defineProperty(document,"compatMode",{configurable:true,value:"CSS1Compat"});
});
const disposals:(()=>Promise<void>)[]=[];
afterEach(async()=>{for(const dispose of disposals.splice(0))await dispose();vi.unstubAllGlobals();});
async function setup<D>(adapter:HbsDocumentAdapter<D>,initial:D) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  let saved:DocumentSnapshot<D>={version:0,saved:initial,baseline:initial},preserved:DocumentCheckpoint<D>|undefined;
  const session=await HbsSession.open({adapter,requestId:n=>`r-${n}`,preservation:{load:()=>preserved,save:value=>{preserved=value;}},port:{
    read:async()=>saved,commit:async request=>{if(request.expectedVersion!==saved.version)throw new DocumentVersionConflict(saved,request.id);
      saved={version:saved.version+1,saved:request.saved,baseline:request.baseline};return saved;}}});
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  disposals.push(async()=>{await act(async()=>root.unmount());session.dispose();host.remove();});
  return {session,adapter,host,root,get saved(){return saved;},external:async(value:D)=>{saved={...saved,version:saved.version+1,saved:value};await act(async()=>session.syncLatest());}};
}
async function frame(){await act(async()=>{await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));});}
async function click(host:HTMLElement,label:string){const button=[...host.querySelectorAll("button")].find(button=>button.textContent===label||button.getAttribute("aria-label")===label);
  if(!button)throw new Error(`missing ${label}`);await act(async()=>button.click());}
async function editStructuredField(host:HTMLElement,text:string){
  await click(host,"编辑research_question");
  const input=host.querySelector<HTMLTextAreaElement>('textarea[aria-label="编辑research_question正文"]')!;
  await act(async()=>{
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,text);
    input.dispatchEvent(new Event("input",{bubbles:true}));
  });
  return input;
}
async function commitStructuredField(host:HTMLElement){
  const input=host.querySelector<HTMLTextAreaElement>('textarea[aria-label="编辑research_question正文"]');
  if(!input)throw new Error("missing structured field input");
  await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",ctrlKey:true,bubbles:true})));
}
it("renders pending reading references with only the actual block schema's properties",async()=>{
  const reference={id:noteBlockId("reading-ref"),type:"readingNoteReference" as const,props:{},children:[],
    content:{kind:"reference" as const,targetKind:"reading_note" as const,targetId:"synthetic-reading",labelSnapshot:"Original reference",quoteSnapshot:"original quote"}};
  const initial={...noteFixture,blocks:[...noteFixture.blocks,reference]};
  const f=await setup(createNoteDocumentAdapter(directTextDiff),initial);
  const gateway=new Proxy(noteRendererGateway,{get(target,key){return key==="getReadingNote"
    ?async()=>{throw new Error("synthetic unavailable reading reference");}:Reflect.get(target,key);}});
  await act(async()=>f.root.render(<NotesProvider gateway={gateway}><HbsNoteEditor session={f.session}/></NotesProvider>));await frame();
  await f.external({...initial,blocks:initial.blocks.map(block=>block.id===reference.id?{...reference,content:{...reference.content,labelSnapshot:"Incoming reference"}}:block)});await frame();
  expect(f.host.querySelector('.note-diff-deleted-blocks')?.textContent).toContain("Original reference");
  expect(f.host.querySelector('.ProseMirror')).not.toBeNull();
  expect(f.session.getSnapshot().content.blocks.at(-1)?.content).toMatchObject({targetKind:"reading_note",labelSnapshot:"Incoming reference"});
});
it("changes note highlight configuration without replacing the native view or accepting pending changes",async()=>{
  const f=await setup(createNoteDocumentAdapter(directTextDiff),noteFixture);let editor:NoteEditorInstance|undefined;
  const render=async(color:string)=>{
    await act(async()=>f.root.render(<NotesProvider gateway={noteRendererGateway}>
      <HbsNoteEditor session={f.session} configuration={{highlight:{addedUnderline:color}}} onEditorReady={value=>{editor=value;}}/>
    </NotesProvider>));await frame();
  };
  await render("#1266cc");
  const mounted=editor!.prosemirrorView;
  await f.external({...noteFixture,blocks:noteFixture.blocks.map(block=>block.id==="hbs-inline"
    ?{...block,content:{kind:"inline",items:[{type:"text",text:"行内公式：",styles:[]},{type:"inlineFormula",source:"x^3"}]}}:block)});
  const content=f.session.getSnapshot().content,baseline=f.session.getSnapshot().baselineProjection;
  await render("#8833bb");
  expect(editor!.prosemirrorView).toBe(mounted);
  expect(f.session.getSnapshot().content).toEqual(content);
  expect(f.session.getSnapshot().baselineProjection).toEqual(baseline);
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.host.querySelector('.note-diff-added-atom [data-source="x^3"]')).not.toBeNull();
  expect(f.host.querySelector<HTMLElement>(".editor-diff-highlight")?.style.getPropertyValue("--editor-diff-added-underline")).toBe("#8833bb");
});
it("keeps sparse native schema properties unchanged when undoing an acknowledged Keep",async()=>{
  const sparse={...noteFixture,blocks:noteFixture.blocks.map(block=>({...block,props:{},children:block.children.map(child=>({...child,props:{}}))}))};
  const f=await setup(createNoteDocumentAdapter(directTextDiff),sparse);let editor:NoteEditorInstance|undefined;
  await act(async()=>f.root.render(<NotesProvider gateway={noteRendererGateway}><HbsNoteEditor session={f.session} onEditorReady={value=>{editor=value;}}/></NotesProvider>));await frame();
  const path=["blocks",{key:"id",value:"hbs-title"},"content","items",0,"text"] as const;
  await f.external(f.adapter.replace(f.saved.saved,path,"修改的标题"));await frame();
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  await click(f.host,"保留第 1 处变更");expect(f.session.getSnapshot().changes).toHaveLength(0);
  await act(async()=>{expect(editor!.undo()).toBe(true);});await frame();
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.session.getSnapshot().content).toEqual(f.adapter.replace(sparse,path,"修改的标题"));
  await act(async()=>{expect(editor!.redo()).toBe(true);});await frame();
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.session.getSnapshot().content).toEqual(f.adapter.replace(sparse,path,"修改的标题"));
});
it.each([{text:"1",rich:false},{text:"123",rich:false},{text:"123",rich:true}])("keeps acknowledged ordinary input $text ordinary through native Undo and Redo (rich: $rich)",async({text,rich})=>{
  const paragraph={...noteFixture.blocks.find(block=>block.id==="hbs-score")!,content:{kind:"inline" as const,items:[]}};
  const initial={...noteFixture,blocks:rich?noteFixture.blocks.map(block=>block.id===paragraph.id?paragraph:block):[paragraph]};
  const f=await setup(createNoteDocumentAdapter(directTextDiff),initial);let editor:NoteEditorInstance|undefined;
  await act(async()=>f.root.render(<NotesProvider gateway={noteRendererGateway}><HbsNoteEditor session={f.session} onEditorReady={value=>{editor=value;}}/></NotesProvider>));await frame();
  await act(async()=>editor!.setTextCursorPosition("hbs-score","start"));
  const position=editor!.prosemirrorState.selection.from;
  for(const [index,char] of [...text].entries())await act(async()=>editor!.prosemirrorView.dispatch(editor!.prosemirrorState.tr.insertText(char,position+index)));
  const typed=f.session.getSnapshot().content;
  await act(async()=>f.session.save());
  expect(f.session.getSnapshot().changes).toHaveLength(0);expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  await act(async()=>{expect(editor!.undo()).toBe(true);});await frame();
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  expect(f.session.getSnapshot().content).toEqual(initial);
  expect(f.session.getSnapshot().baselineProjection).toEqual(initial);
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  await act(async()=>f.session.save());expect(f.saved.saved).toEqual(initial);expect(f.saved.baseline).toEqual(initial);
  await act(async()=>{expect(editor!.redo()).toBe(true);});await frame();
  expect(f.session.getSnapshot().content).toEqual(typed);expect(f.session.getSnapshot().baselineProjection).toEqual(typed);
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);expect(f.session.getSnapshot().changes).toHaveLength(0);
  await act(async()=>f.session.save());expect(f.saved.saved).toEqual(typed);expect(f.saved.baseline).toEqual(typed);
});
it("merges native ordinary lines with an incoming inline formula and keeps atom review/history local",async()=>{
  const f=await setup(createNoteDocumentAdapter(directTextDiff),noteFixture);let editor:NoteEditorInstance|undefined;
  await act(async()=>f.root.render(<NotesProvider gateway={noteRendererGateway}><HbsNoteEditor session={f.session} onEditorReady={value=>{editor=value;}}/></NotesProvider>));await frame();
  let afterFormula:number|undefined;
  editor!.prosemirrorState.doc.descendants((node,position)=>{if(node.type.name==="inlineFormula")afterFormula=position+node.nodeSize;});
  if(afterFormula===undefined)throw new Error("missing inline formula");
  const position=afterFormula;
  await act(async()=>{
    editor!.prosemirrorView.dispatch(editor!.prosemirrorState.tr.insertText("\n3123",position));
    editor!.prosemirrorView.dispatch(editor!.prosemirrorState.tr.insertText("\n123213",position+5));
  });
  const path=["blocks",{key:"id",value:"hbs-inline"},"content","items",1] as const;
  await f.external(f.adapter.replace(f.saved.saved,path,{type:"inlineFormula",source:"x^{106}"}));await frame();
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  expect(JSON.stringify(f.session.getSnapshot().content)).toContain("3123");
  expect(JSON.stringify(f.session.getSnapshot().baselineProjection)).toContain("3123");
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  const current=f.host.querySelector('[data-id="hbs-inline"] .note-diff-added-atom');
  expect(current?.querySelector('[data-inline-content-type="inlineFormula"]')?.getAttribute("data-source")).toBe("x^{106}");
  expect(current?.querySelector(".katex")).not.toBeNull();
  expect(f.host.querySelector('[data-id="hbs-inline"] .note-diff-added-text')).toBeNull();
  await act(async()=>{expect(editor!.undo()).toBe(true);});
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  expect(f.adapter.read(f.session.getSnapshot().content,path)).toEqual({type:"inlineFormula",source:"x^{106}"});
  expect(JSON.stringify(f.session.getSnapshot().content)).not.toContain("3123");
  await act(async()=>{expect(editor!.redo()).toBe(true);});
  expect(JSON.stringify(f.session.getSnapshot().content)).toContain("3123");expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  await click(f.host,"保留第 1 处变更");expect(f.session.getSnapshot().changes).toHaveLength(0);
  await act(async()=>{expect(editor!.undo()).toBe(true);});expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.host.querySelector('.note-diff-added-atom [data-source="x^{106}"]')).not.toBeNull();
});
it("aligns a native nested ordinary edit before review and Undo restores the displayed red text", async () => {
  const f = await setup(createNoteDocumentAdapter(directTextDiff), noteFixture); let editor: NoteEditorInstance | undefined;
  await act(async () => f.root.render(<NotesProvider gateway={noteRendererGateway}><HbsNoteEditor session={f.session} onEditorReady={value => { editor = value; }}/></NotesProvider>)); await frame();
  const path = ["blocks", {key:"id",value:"hbs-method"}, "children", {key:"id",value:"hbs-nested-a"}, "content", "items", 0, "text"] as const;
  await act(async () => editor!.updateBlock("hbs-nested-a", {content:"重复实验 143 次。"}));
  await f.external(f.adapter.replace(f.saved.saved, path, "外部修改 v150")); await frame();
  expect(f.adapter.read(f.saved.baseline,path)).toBe("重复实验 143 次。");
  expect(f.adapter.read(f.session.getSnapshot().content,path)).toBe("外部修改 v150");
  expect(f.host.querySelector(".note-diff-deleted-blocks")?.textContent).toContain("重复实验 143 次。");
  expect(f.host.querySelector('[aria-label="incoming"]')).toBeNull();
  await click(f.host,"撤销第 1 处变更");
  expect(f.adapter.read(f.session.getSnapshot().content,path)).toBe("重复实验 143 次。");
  await act(async () => { expect(editor!.undo()).toBe(true); });
  expect(f.adapter.read(f.session.getSnapshot().content,path)).toBe("外部修改 v150");
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
});
it("uses the same red baseline for structured field review and native history", async () => {
  const adapter = createStructuredDocumentAdapter(directTextDiff);
  const path = ["fields", {key:"field",value:"research_question"}, "statements", {key:"id",value:"hbs-statement-0"}, "text"] as const;
  const initial = adapter.replace(structuredFixture,path,"重复实验 3 次。");
  const f = await setup(adapter,initial);
  await act(async () => f.root.render(<HbsStructuredEditor session={f.session}/>));
  const editor = EditorView.findFromDOM(f.host.querySelector<HTMLElement>('[hidden] .cm-editor')!)!;
  await editStructuredField(f.host,"重复实验 143 次。");
  await commitStructuredField(f.host);
  await f.external(adapter.replace(f.saved.saved,path,"外部修改 v150"));
  expect(adapter.read(f.saved.baseline,path)).toBe("重复实验 143 次。");
  expect(f.host.querySelector('[aria-label="修改前"]')?.textContent).toContain("重复实验 143 次。");
  expect(f.host.querySelector('[aria-label="incoming"]')).toBeNull();
  await click(f.host,"Undo"); expect(adapter.read(f.session.getSnapshot().content,path)).toBe("重复实验 143 次。");
  await act(async () => { expect(undo(editor)).toBe(true); });
  expect(adapter.read(f.session.getSnapshot().content,path)).toBe("外部修改 v150"); expect(f.session.getSnapshot().conflicts).toHaveLength(0);
});
it("does not reapply an acknowledged ordinary edit when the compatibility editor undoes typing", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const initial = "sample 120\n\nstable\n\nscore 80%", unit = (text: string) => [{id:"text",label:"text",text}];
  const repository = new MemoryHbsRepository({version:1,saved:unit(initial),baseline:unit(initial)});
  const authority = new HbsWireAuthority(repository), policy = createCodeMirrorHbsPolicy();
  const port = await HbsWireClient.connect({request: wire => authority.handle(wire)}, {type:"patch",calculate:policy.calculate});
  const client = await HbsClient.open({port,documentId:"text",policy,requestId:n=>`native-${n}`});
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  disposals.push(async () => { await act(async () => root.unmount()); client.dispose(); port.dispose(); host.remove(); });
  await act(async () => root.render(<HbsTextEditor client={client} path="H.txt"/>));
  const editor = EditorView.findFromDOM(host.querySelector<HTMLElement>(".cm-editor")!)!;
  await act(async () => editor.dispatch({changes:{from:7,to:10,insert:"150"},userEvent:"input.type"}));
  const snapshot = repository.read(); await repository.commit(snapshot.version,[{id:"text",saved:initial.replace("80%","85%"),baseline:initial}]);
  await act(async () => client.syncLatest());
  expect(repository.read().baseline[0]!.text).toContain("150");
  await act(async () => { expect(undo(editor)).toBe(true); });
  expect(editor.state.doc.toString()).toContain("sample 120");
  expect(editor.state.doc.toString()).toContain("85%"); expect(client.getIncoming()).toHaveLength(0);
  await act(async () => { expect(redo(editor)).toBe(true); });
  expect(editor.state.doc.toString()).toContain("sample 150");
});
it.each([false, true])("groups adjacent formula review controls by their action capability, conflict: %s", async conflict => {
  const f = await setup(createNoteDocumentAdapter(directTextDiff), noteFixture);
  await act(async () => f.root.render(<NotesProvider gateway={noteRendererGateway}><HbsNoteEditor session={f.session}/></NotesProvider>));
  await frame();
  const blockPath = ["blocks", { key: "id", value: "hbs-formula" }, "content", "source"] as const;
  const inlinePath = ["blocks", { key: "id", value: "hbs-inline" }, "content", "items", 1] as const;
  const updated = f.adapter.replace(f.adapter.replace(noteFixture, blockPath, "E = 120mc^2"), inlinePath, { type: "inlineFormula", source: "x^{119}" });
  await f.external(updated); await frame();
  if (conflict) {
    await act(async () => f.session.edit(f.adapter.replace(f.session.getSnapshot().content, inlinePath, { type: "inlineFormula", source: "x^{mine}" })));
    await f.external(f.adapter.replace(updated, inlinePath, { type: "inlineFormula", source: "x^{incoming}" }));
    await frame();
  }
  const blockCurrent = f.host.querySelector('[data-id="hbs-formula"]')!;
  const blockActions = f.host.querySelector('button[aria-label="保留第 1 处变更"]')!.closest(".note-diff-chunk-actions")!;
  const inlineOld = f.host.querySelector(`[data-block-review-anchor-for="${conflict ? "hbs-inline" : "hbs-formula"}"]`)!;
  const inlineCurrent = f.host.querySelector('[data-id="hbs-inline"]')!;
  expect(blockCurrent.compareDocumentPosition(blockActions) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(inlineOld.compareDocumentPosition(inlineCurrent) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  if (conflict) {
    expect(blockActions.compareDocumentPosition(inlineOld) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(f.host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(2);
    const incoming = f.host.querySelector('[aria-label="incoming"]')!;
    expect(inlineOld.compareDocumentPosition(incoming) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(incoming.compareDocumentPosition(inlineCurrent) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(incoming.getAttribute("contenteditable")).toBe("false");
  } else {
    expect(inlineCurrent.compareDocumentPosition(blockActions) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(f.host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(1);
    await click(f.host, "保留第 1 处变更");
    expect(f.adapter.read(f.saved.baseline, blockPath)).toBe("E = 120mc^2");
    expect(f.adapter.read(f.saved.baseline, inlinePath)).toEqual({ type: "inlineFormula", source: "x^{119}" });
    expect(f.session.getSnapshot().changes).toHaveLength(0);
  }
});
it("keeps a deleted block before its changed successor with shared controls after the group", async () => {
  const f = await setup(createNoteDocumentAdapter(directTextDiff), noteFixture);
  await act(async () => f.root.render(<NotesProvider gateway={noteRendererGateway}><HbsNoteEditor session={f.session}/></NotesProvider>));
  await frame();
  const updated = f.adapter.replace(noteFixture, ["blocks", { key: "id", value: "hbs-inline" }, "content", "items", 1],
    { type: "inlineFormula", source: "x^{119}" });
  await f.external({ ...updated, blocks: updated.blocks.filter(block => block.id !== "hbs-formula") });
  await frame();
  const deleted = f.host.querySelector<HTMLElement>('.note-diff-deleted-blocks[data-hbs-review-group]')!;
  const actions = f.host.querySelector('button[aria-label="保留第 1 处变更"]')!.closest(".note-diff-chunk-actions")!;
  expect(deleted).toBeDefined();
  expect(deleted.compareDocumentPosition(actions) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  const originals = deleted.querySelectorAll(":scope > .bn-block-group > *");
  expect(originals).toHaveLength(2);
  expect(originals[0]!.querySelector("annotation")?.textContent).toBe("E = mc^2");
  expect(originals[1]!.textContent).toContain("行内公式：");
  const current = f.host.querySelector('[data-id="hbs-inline"]')!;
  expect(current.compareDocumentPosition(actions) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(f.host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(1);
});
it("marks whole block formulas through conflict choice and native review undo/redo", async () => {
  const f = await setup(createNoteDocumentAdapter(directTextDiff), noteFixture);
  let editor: NoteEditorInstance | undefined;
  await act(async () => f.root.render(<NotesProvider gateway={noteRendererGateway}>
    <HbsNoteEditor session={f.session} onEditorReady={value => { editor = value; }}/>
  </NotesProvider>));
  await frame();
  if (!editor) throw new Error("missing note editor");
  const native = editor;
  const path = ["blocks", { key: "id", value: "hbs-formula" }, "content", "source"] as const;
  await f.external(f.adapter.replace(noteFixture, path, "E = 139mc^2"));
  await act(async () => { native.updateBlock("hbs-formula", { props: { source: "E = 140mc^2" } }); });
  await f.external(f.adapter.replace(f.saved.saved, path, "E = 141mc^2"));
  await frame();
  const formula = (scope: string) => f.host.querySelector(`${scope} .note-formula-preview`);
  expect(formula('.note-diff-deleted-blocks:not(.hbs-note-incoming) .note-diff-deleted-block-atom')?.getAttribute("aria-label")).toBe("E = mc^2");
  const incoming = formula('.hbs-note-incoming .note-diff-deleted-block-atom');
  expect(incoming?.getAttribute("aria-label")).toBe("E = 141mc^2");
  expect(incoming?.closest('[contenteditable="false"]')).not.toBeNull();
  expect(formula('.note-diff-added-block-atom')?.getAttribute("aria-label")).toBe("E = 140mc^2");
  expect(f.host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(1);
  await click(f.host, "Take");
  expect(f.adapter.read(f.session.getSnapshot().content, path)).toBe("E = 141mc^2");
  expect(f.adapter.read(f.saved.baseline, path)).toBe("E = mc^2");
  await act(async () => { expect(native.undo()).toBe(true); }); await frame();
  expect(formula('.hbs-note-incoming .note-diff-deleted-block-atom')?.getAttribute("aria-label")).toBe("E = 141mc^2");
  expect(formula('.note-diff-added-block-atom')?.getAttribute("aria-label")).toBe("E = 140mc^2");
  await act(async () => { expect(native.redo()).toBe(true); }); await frame();
  await click(f.host, "保留第 1 处变更");
  expect(f.adapter.read(f.saved.baseline, path)).toBe("E = 141mc^2");
  expect(f.host.querySelector('.note-diff-added-block-atom,.note-diff-deleted-block-atom')).toBeNull();
  await act(async () => { expect(native.undo()).toBe(true); }); await frame();
  expect(formula('.note-diff-added-block-atom')?.getAttribute("aria-label")).toBe("E = 141mc^2");
});
it("uses actual BlockNote cells, native review history and lossless ordinary/new projection",async()=>{
  const f=await setup(createNoteDocumentAdapter(directTextDiff),noteFixture);let editor:NoteEditorInstance|undefined;
  await act(async()=>f.root.render(<NotesProvider gateway={noteRendererGateway}><HbsNoteEditor session={f.session} onEditorReady={value=>{editor=value;}}/></NotesProvider>));await frame();
  const tablePath=["blocks",{key:"id",value:"hbs-table"},"content","rows",1,1] as const;
  await f.external(f.adapter.replace(noteFixture,tablePath,{kind:"inline",items:[{type:"text",text:"0.99",styles:[]}]}));await frame();
  expect(f.host.querySelector(".note-diff-deleted-tableCell")?.textContent).toContain("0.80");
  expect(f.host.querySelectorAll(".bn-editor")).toHaveLength(1);
  await click(f.host,"保留第 1 处变更");expect(f.session.getSnapshot().changes).toHaveLength(0);
  await act(async()=>{expect(editor!.undo()).toBe(true);});expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.adapter.read(f.saved.baseline,tablePath)).toEqual(f.adapter.read(noteFixture,tablePath));
  await act(async()=>{expect(editor!.redo()).toBe(true);});expect(f.session.getSnapshot().changes).toHaveLength(0);
  await act(async()=>{editor!.updateBlock("hbs-next",{content:"ordinary typed"});});
  expect(f.session.getSnapshot().changes).toHaveLength(0);await act(async()=>f.session.save());
  expect(f.saved.saved).toEqual(f.saved.baseline);
});
it("shares structured cards with actual statement objects and native conflict/history",async()=>{
  const f=await setup(createStructuredDocumentAdapter(directTextDiff),structuredFixture);
  await act(async()=>f.root.render(<HbsStructuredEditor session={f.session}/>));
  const card = f.host.querySelector('[data-field-id="research_question"]')!;
  expect(card.getAttribute("data-appearance")).toBe("reading");
  expect(card.querySelector(".text-editor-toolbar, .cm-gutters, footer")).toBeNull();
  const path=["fields",{key:"field",value:"research_question"},"statements",{key:"id",value:"hbs-statement-0"},"text"] as const;
  await f.external(f.adapter.replace(structuredFixture,path,"incoming one 85%"));
  const surface=f.host.querySelector<HTMLElement>('[hidden] .cm-editor')!;
  const editor=EditorView.findFromDOM(surface)!;
  await editStructuredField(f.host,"my candidate 91%");await commitStructuredField(f.host);
  await f.external(f.adapter.replace(f.saved.saved,path,"incoming two 90%"));
  expect(f.host.querySelector('[aria-label="incoming"]')?.textContent).toContain("90%");
  await click(f.host,"Take");expect(f.adapter.read(f.session.getSnapshot().content,path)).toContain("90%");
  await act(async()=>{expect(undo(editor)).toBe(true);});expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0); expect(f.host.querySelectorAll('[aria-label="incoming"]')).toHaveLength(1);
  await act(async()=>{expect(redo(editor)).toBe(true);});expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  await click(f.host,"Keep");expect(f.adapter.read(f.saved.baseline,path)).toContain("90%");
  await act(async()=>{expect(undo(editor)).toBe(true);});expect(f.adapter.read(f.saved.baseline,path)).toEqual(f.adapter.read(structuredFixture,path));
});
it("shows lossless raw snapshots without rendering their contents or mounting editors",async()=>{
  const f=await setup(createStructuredDocumentAdapter(directTextDiff),structuredFixture);
  const value={...structuredFixture,extra:"<table><tr><td>原始数据</td></tr></table>\n第二行"};
  await act(async()=>f.root.render(<HbsSnapshotSource value={value}/>));
  expect(JSON.parse(f.host.querySelector("pre")!.textContent!)).toEqual(value);
  expect(f.host.querySelector("table, .cm-editor, textarea, button")).toBeNull();
  await act(async()=>f.root.render(<HbsSnapshotSource value={"第一行\n\n第二行  "} text/>));
  expect(f.host.querySelector("pre")!.textContent).toBe("第一行\n\n第二行  ");
});

it("shows actual SQLite columns without decoding JSON or mounting editors",async()=>{
  const f=await setup(createStructuredDocumentAdapter(directTextDiff),structuredFixture);
  const raw=' {"text":"<b>原文</b>","items": []} ';
  await act(async()=>f.root.render(<HbsStorageRows tables={[{table:"note_blocks",columns:["id","content_json","trashed_at"],rows:[{id:"block-1",content_json:raw,trashed_at:null}]}]}/>));
  expect(f.host.querySelector("td:nth-child(2) pre")!.textContent).toBe(raw);
  expect(f.host.querySelector("td:nth-child(3)")!.textContent).toBe("NULL");
  expect(f.host.querySelector("b,.cm-editor,.bn-editor,textarea,button")).toBeNull();
});

it("uses native text chunk boundaries for Both and keeps an unrelated diff through history",async()=>{
  const initial="title\n\nscore 80%\n\nmethod old\n\nnext old";
  const f=await setup(createTextDocumentAdapter(directTextDiff),initial);
  await act(async()=>f.root.render(<HbsLeafEditor session={f.session} path={[]} label="H.txt"/>));
  await f.external(initial.replace("80%","85%"));
  const editor=EditorView.findFromDOM(f.host.querySelector<HTMLElement>(".cm-editor")!)!;
  const offset=editor.state.doc.toString().indexOf("85%");
  await act(async()=>editor.dispatch({changes:{from:offset,to:offset+3,insert:"91%"},userEvent:"input.type"}));
  await f.external(f.saved.saved.replace("85%","90%").replace("next old","next new"));
  const widgets=f.host.querySelectorAll('[aria-label="incoming"]');expect(widgets).toHaveLength(1);
  expect(widgets[0]!.textContent).toContain("score 90%");expect(widgets[0]!.textContent).not.toContain("title");
  await click(f.host,"Both");
  expect(f.session.getSnapshot().content).toContain("score 90%\nscore 91%");
  expect(f.session.getSnapshot().content.match(/title/gu)).toHaveLength(1);
  expect(f.session.getSnapshot().content).toContain("next new");
  await act(async()=>{expect(undo(editor)).toBe(true);});
  expect(f.session.getSnapshot().conflicts).toHaveLength(1);expect(f.session.getSnapshot().content).toContain("next new");
  await act(async()=>{expect(redo(editor)).toBe(true);});
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);expect(f.session.getSnapshot().baselineProjection).toBe(initial);
});

it("projects ordinary edits within a structured statement independently of pending new",async()=>{
  const f=await setup(createStructuredDocumentAdapter(directTextDiff),structuredFixture);
  const path=["fields",{key:"field",value:"research_question"},"statements",{key:"id",value:"hbs-statement-0"},"text"] as const;
  const original=f.adapter.replace(structuredFixture,path,"score 80%\n\nmethod old");
  await f.external(original);await act(async()=>f.session.review(original,original));
  await act(async()=>f.root.render(<HbsStructuredEditor session={f.session}/>));
  await f.external(f.adapter.replace(f.saved.saved,path,"score 85%\n\nmethod old"));
  await editStructuredField(f.host,"score 91%\n\nmethod typed");await commitStructuredField(f.host);
  expect(f.adapter.read(f.saved.saved,path)).toBe("score 91%\n\nmethod typed");
  expect(f.adapter.read(f.saved.baseline,path)).toBe("score 80%\n\nmethod typed");
});

it("renders inserted blocks without a whole-note old snapshot and groups conflicts in one real block",async()=>{
  const adapter=createNoteDocumentAdapter(directTextDiff);
  const path=["blocks",{key:"id",value:"hbs-score"},"content","items",0,"text"] as const;
  const initial=adapter.replace(noteFixture,path,"left 10 middle 20 right");
  const f=await setup(adapter,initial);let editor:NoteEditorInstance|undefined;
  await act(async()=>f.root.render(<NotesProvider gateway={noteRendererGateway}><HbsNoteEditor session={f.session} onEditorReady={value=>{editor=value;}}/></NotesProvider>));await frame();
  const added={...initial,blocks:[...initial.blocks,{id:noteBlockId("new-paragraph"),type:"paragraph" as const,props:{},content:{kind:"inline" as const,items:[{type:"text" as const,text:"added block",styles:[]}]},children:[]}]};
  await f.external(added);await frame();
  expect(f.host.querySelector(".note-diff-deleted-blocks")).toBeNull();
  await f.external(adapter.replace(f.saved.saved,path,"left 11 middle 21 right"));
  await act(async()=>editor!.updateBlock("hbs-score",{content:"left 15 middle 25 right"}));
  await f.external(adapter.replace(f.saved.saved,path,"left 12 middle 22 right"));await frame();
  expect(f.session.getSnapshot().conflicts).toHaveLength(1);
  expect(f.host.querySelectorAll('[aria-label="incoming"]')).toHaveLength(1);
  await click(f.host,"Take");
  expect(adapter.read(f.session.getSnapshot().content,path)).toBe("left 12 middle 22 right");
  await act(async()=>{expect(editor!.undo()).toBe(true);});
  expect(adapter.read(f.session.getSnapshot().content,path)).toBe("left 15 middle 25 right");
  expect(f.host.querySelectorAll('[aria-label="incoming"]')).toHaveLength(1);
});

it("shows a removed block at its sibling boundary without marking the surviving note as new",async()=>{
  const f=await setup(createNoteDocumentAdapter(directTextDiff),noteFixture);
  await act(async()=>f.root.render(<NotesProvider gateway={noteRendererGateway}><HbsNoteEditor session={f.session}/></NotesProvider>));await frame();
  await f.external({...noteFixture,blocks:noteFixture.blocks.filter(block=>block.id!=="hbs-score")});await frame();
  expect(f.host.querySelectorAll('.note-diff-added-block')).toHaveLength(0);
  expect(f.host.querySelector('.note-diff-deleted-blocks')?.textContent).toContain("80%");
  expect(f.host.querySelector('.note-diff-deleted-blocks')?.textContent).not.toContain("研究计划");
  await click(f.host,"撤销第 1 处变更");
  expect(f.session.getSnapshot().content.blocks.map(block=>block.id)).toEqual(noteFixture.blocks.map(block=>block.id));
});

it("repeats committed conflict Keep undo/redo through CodeMirror without stale incoming blocking it",async()=>{
  const initial="title\n\nscore 80%\n\nmethod old\n\nnext old";
  const f=await setup(createTextDocumentAdapter(directTextDiff),initial);
  await act(async()=>f.root.render(<HbsLeafEditor session={f.session} path={[]} label="H.txt"/>));
  await f.external(initial.replace("80%","85%"));
  const editor=EditorView.findFromDOM(f.host.querySelector<HTMLElement>(".cm-editor")!)!;
  const from=editor.state.doc.toString().indexOf("85%");
  await act(async()=>editor.dispatch({changes:{from,to:from+3,insert:"91%"},userEvent:"input.type"}));
  await f.external(f.saved.saved.replace("85%","90%"));
  await click(f.host,"Keep");expect(f.saved.baseline).toContain("91%");
  await act(async()=>{expect(undo(editor)).toBe(true);});expect(f.saved.baseline).toContain("80%");
  expect(f.session.getSnapshot().conflicts).toHaveLength(1);
  await act(async()=>{expect(redo(editor)).toBe(true);});expect(f.saved.baseline).toContain("91%");
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  await act(async()=>{expect(undo(editor)).toBe(true);});expect(f.saved.baseline).toContain("80%");
  expect(f.saved.saved).toContain("90%");expect(editor.state.doc.toString()).toContain("91%");
});

it.each([false,true])("retains saved human table-cell new and native conflict history with comparison configured: %s",async compare=>{
  const f=await setup(createNoteDocumentAdapter(directTextDiff),noteFixture);let editor:NoteEditorInstance|undefined;
  await act(async()=>f.root.render(<NotesProvider gateway={noteRendererGateway}><HbsNoteEditor session={f.session} {...(compare?{tableComparison:compareTableRows}:{})} onEditorReady={value=>{editor=value;}}/></NotesProvider>));await frame();
  const path=["blocks",{key:"id",value:"hbs-table"},"content","rows",1,1] as const;
  const cell=(text:string)=>({kind:"inline",items:[{type:"text",text,styles:[]}]});
  await f.external(f.adapter.replace(f.saved.saved,path,cell("0.99")));await frame();
  await act(async()=>{
    let from:number|undefined;
    editor!.prosemirrorState.doc.descendants((node,position)=>{if(node.isText&&node.text==="0.99")from=position;});
    if(from===undefined)throw new Error("missing real table-cell text");
    editor!.prosemirrorView.dispatch(editor!.prosemirrorState.tr.insertText("0.88",from,from+4));
  });
  await act(async()=>f.session.save());expect(f.adapter.read(f.saved.baseline,path)).toEqual(f.adapter.read(noteFixture,path));
  await f.external(f.adapter.replace(f.saved.saved,path,cell("0.44")));await frame();
  expect(f.adapter.read(f.session.getSnapshot().content,path)).toEqual(cell("0.88"));
  expect(f.session.getSnapshot().conflicts).toHaveLength(1);
  const incoming=f.host.querySelector('[aria-label="incoming"]');
  expect(incoming?.textContent).toBe("0.44");expect(incoming?.closest("td,th")).not.toBeNull();
  await click(f.host,"Take");expect(f.adapter.read(f.session.getSnapshot().content,path)).toEqual(cell("0.44"));
  await act(async()=>{expect(editor!.undo()).toBe(true);});
  expect(f.adapter.read(f.session.getSnapshot().content,path)).toEqual(cell("0.88"));expect(f.session.getSnapshot().conflicts).toHaveLength(1);
  await click(f.host,"Keep");expect(f.adapter.read(f.saved.baseline,path)).toEqual(cell("0.88"));
});

it("retains the production read-only history guard without replacing the HBS note editor",async()=>{
  const f=await setup(createNoteDocumentAdapter(directTextDiff),noteFixture);let editor:NoteEditorInstance|undefined;
  const render=async(isReadOnly:boolean)=>{await act(async()=>f.root.render(<NotesProvider gateway={noteRendererGateway}>
    <HbsNoteEditor session={f.session} isReadOnly={isReadOnly} onEditorReady={value=>{editor=value;}}/>
  </NotesProvider>));await frame();};
  await render(false);
  await act(async()=>editor!.updateBlock(noteFixture.blocks[1]!.id,{content:"ordinary input"}));
  const current=f.session.getSnapshot().content, originalEditor=editor;
  await render(true);await act(async()=>{editor!.undo();});
  expect(editor).toBe(originalEditor);expect(f.session.getSnapshot().content).toEqual(current);
  expect(f.saved.version).toBe(0);
  await render(false);await act(async()=>{editor!.undo();});
  expect(f.session.getSnapshot().content).toEqual(noteFixture);
  await act(async()=>editor!.redo());
  await act(async()=>{holdTextEditor(noteFixture.pageId,"agent-test",true);editor!.undo();});
  expect(f.session.getSnapshot().content).toEqual(current);
  await act(async()=>{holdTextEditor(noteFixture.pageId,"agent-test",false);});
  await act(async()=>editor!.undo());expect(f.session.getSnapshot().content).toEqual(noteFixture);
});

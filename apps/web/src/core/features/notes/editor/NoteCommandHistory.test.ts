// @vitest-environment happy-dom
import {BlockNoteEditor,createExtension} from "@blocknote/core";
import {Plugin} from "@tiptap/pm/state";
import {afterEach,expect,it,vi} from "vitest";
import {NoteCommandHistory} from "./NoteCommandHistory";
import {noteCommandHistoryPlugin} from "./noteCommandHistoryPlugin";
import type {NoteEditorInstance} from "./noteEditorSchema";
const stops:(()=>void)[]=[];afterEach(()=>stops.splice(0).forEach(stop=>stop()));
function setup(history=new NoteCommandHistory()){
  const editor:BlockNoteEditor=BlockNoteEditor.create({initialContent:[{id:"body",type:"paragraph",content:"before"}],extensions:[createExtension(()=>({key:"command-history-test",prosemirrorPlugins:[noteCommandHistoryPlugin(history,()=>editor as unknown as NoteEditorInstance,()=>true)]}))()]});
  const host=document.createElement("div");document.body.append(host);editor.mount(host);stops.push(()=>{editor.unmount();host.remove();});
  return{history,editor,text:()=>JSON.stringify(editor.getBlock("body")!.content)};
}
it("orders native text and async lifecycle commands in the same chain without replacing the editor",async()=>{
  const {history,editor,text}=setup();editor.updateBlock("body",{content:"after"});
  const undo=vi.fn(),redo=vi.fn();history.closeGroups();history.record({id:"delete",undo,redo});
  await history.run("undo");expect(undo).toHaveBeenCalledOnce();expect(text()).toContain("after");
  await history.run("undo");expect(text()).toContain("before");
  await history.run("redo");expect(text()).toContain("after");
  await history.run("redo");expect(redo).toHaveBeenCalledOnce();
});
it("retains command ownership through PM plugin reconfiguration and keeps native grouping",async()=>{
  const {history,editor,text}=setup();editor.updateBlock("body",{content:"one"});editor.updateBlock("body",{content:"two"});
  editor.registerExtension({key:"probe",prosemirrorPlugins:[new Plugin({})]});editor.unregisterExtension("probe");await Promise.resolve();
  await history.run("undo");expect(text()).toContain("before");
  await history.run("redo");expect(text()).toContain("two");
});
it("does not consume a failed async command and clears redo after a new edit",async()=>{
  const {history,editor}=setup(),undo=vi.fn().mockRejectedValueOnce(new Error("failed")).mockResolvedValue(undefined);
  history.record({id:"delete",undo,redo:vi.fn()});await expect(history.run("undo")).rejects.toThrow("failed");
  await history.run("undo");expect(undo).toHaveBeenCalledTimes(2);
  editor.updateBlock("body",{content:"new edit"});expect(await history.run("redo")).toBe(false);
});
it("keeps text undo scoped to its focused owner while navigation can replay lifecycle commands",async()=>{
  const history=new NoteCommandHistory(),left=setup(history),right=setup(history);
  left.editor.updateBlock("body",{content:"left edit"});right.editor.updateBlock("body",{content:"right edit"});
  await history.run("undo",undefined,left.editor.prosemirrorView.dom);
  expect(left.text()).toContain("before");expect(right.text()).toContain("right edit");
  right.editor.updateBlock("body",{content:"another right edit"});
  await history.run("redo",undefined,left.editor.prosemirrorView.dom);expect(left.text()).toContain("left edit");
  expect(await history.run("undo",undefined,document.body)).toBe(false);
  const undo=vi.fn();history.closeGroups();history.record({id:"delete",undo,redo:vi.fn()});
  await history.run("undo",undefined,document.body);expect(undo).toHaveBeenCalledOnce();expect(right.text()).toContain("another right edit");
});

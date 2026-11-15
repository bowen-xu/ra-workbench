// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {AgentMarkdown} from "./AgentMarkdown";
import {BlockNoteEditor} from "@blocknote/core";
import {baseBlockEditorSchema} from "../../components/block-editor/baseSchema";
import {blockEditorDefaults} from "../../components/block-editor/defaults";
import {closeHistory} from "@tiptap/pm/history";
import {AgentUserMessage} from "./AgentUserMessage";
import {insertAgentReference, serializeAgentSourceReferences, type AgentInputDraft} from "@research-assistant/core/agent";
import {readComposerClipboard} from "./agentComposerClipboard";
import {BlockEditor} from "../../components/block-editor/BlockEditor";

const cleanups: (()=>Promise<void>)[]=[];
vi.hoisted(()=>{
 if(!document.doctype)document.insertBefore(document.implementation.createDocumentType("html","",""),document.documentElement);
 Object.defineProperty(document,"compatMode",{configurable:true,value:"CSS1Compat"});
});
afterEach(async()=>{for(const release of cleanups.splice(0))await release();document.getSelection()?.removeAllRanges();vi.unstubAllGlobals();});
function copy(range:Range) {
 const selection=document.getSelection()!;selection.removeAllRanges();selection.addRange(range);
 const data=new DataTransfer(),event=new Event("copy",{bubbles:true,cancelable:true});Object.defineProperty(event,"clipboardData",{value:data});
 document.dispatchEvent(event);return {data,event};
}
async function mount(content:React.ReactNode) {
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 await act(async()=>root.render(content));
 cleanups.push(async()=>{await act(async()=>root.unmount());host.remove();});
 return {host,root};
}
it("native selection copy writes Markdown and HTML which paste to editable formulas in one undo step",async()=>{
 const {host}=await mount(<AgentMarkdown value={"**评分**：\n\n\\[\na\n=\nb\n\\]\n\n其中 \\(r\\) 是候选。"}/>);
 const body=host.querySelector<HTMLElement>('.writing-agent-markdown')!,range=document.createRange();range.selectNodeContents(body);
 const {data,event}=copy(range);
 expect(event.defaultPrevented).toBe(true);
 expect(data.getData("text/plain")).toContain("**评分**");
 expect(data.getData("text/plain")).toContain("\\[\na\n=\nb\n\\]");
 expect(data.getData("text/plain")).toContain("\\(r\\)");
 const parsed=document.createElement("div");parsed.innerHTML=data.getData("text/html");expect(parsed.querySelectorAll("math")).toHaveLength(2);
 const editor=BlockNoteEditor.create({...blockEditorDefaults,schema:baseBlockEditorSchema,initialContent:[{id:"body",type:"paragraph",content:[]}]});
 await mount(<BlockEditor editor={editor} onChange={()=>{}}/>);
 editor.setTextCursorPosition("body","start");
 const before=JSON.stringify(editor.document),view=editor.prosemirrorView;
 view.dispatch(closeHistory(view.state.tr));
 const fallback=vi.fn(),paste=new ClipboardEvent("paste",{clipboardData:data});
 await act(async()=>{expect(blockEditorDefaults.pasteHandler({editor,event:paste,defaultPasteHandler:fallback})).toBe(true);});
 expect(fallback).not.toHaveBeenCalled();
 const after=JSON.stringify(editor.document);
 expect(after).toContain('"type":"formulaBlock"');expect(after).toContain('"type":"inlineFormula"');
 await act(async()=>{editor.undo();});expect(JSON.stringify(editor.document)).toBe(before);
 await act(async()=>{editor.redo();});expect(JSON.stringify(editor.document)).toBe(after);
});
it("normal plain-Markdown paste recognizes formulas, while code and ordinary HTML keep native precedence",()=>{
 const plain=new DataTransfer();plain.setData("text/plain","# 标题\n\n文字 \\(x_i\\) 后文\n\n\\[\na\n=\nb\n\\]");
 const pasteHTML=vi.fn(),fallback=vi.fn(()=>true);
 const event=new ClipboardEvent("paste",{clipboardData:plain});
 blockEditorDefaults.pasteHandler({event,editor:{pasteHTML},defaultPasteHandler:fallback});
 expect(pasteHTML.mock.lastCall?.[0]).toContain('data-inline-content-type="inlineFormula"');
 expect(pasteHTML.mock.lastCall?.[0]).toContain('data-content-type="formulaBlock"');
 pasteHTML.mockClear();plain.setData("text/html","<p>HTML wins</p>");
 blockEditorDefaults.pasteHandler({event,editor:{pasteHTML},defaultPasteHandler:fallback});
 expect(pasteHTML).not.toHaveBeenCalled();expect(fallback).toHaveBeenCalledWith({prioritizeMarkdownOverHTML:false});
 plain.clearData("text/html");fallback.mockClear();
 blockEditorDefaults.pasteHandler({event,editor:{pasteHTML,getTextCursorPosition:()=>({block:{type:"codeBlock"}})},defaultPasteHandler:fallback});
 expect(pasteHTML).not.toHaveBeenCalled();expect(fallback).toHaveBeenCalledOnce();
 for(const type of ["blocknote/html","vscode-editor-data","Files"]) {
  plain.setData(type,"owned");pasteHTML.mockClear();
  blockEditorDefaults.pasteHandler({event,editor:{pasteHTML},defaultPasteHandler:fallback});
  expect(pasteHTML).not.toHaveBeenCalled();plain.clearData(type);
 }
});
it("cross-message copy retains user reference identities and rich assistant formulas",async()=>{
 const draft=insertAgentReference({text:"read ",bindings:[]} as AgentInputDraft,{type:"mention",name:"文件",target:{kind:"file",sourceId:"paper",resourceId:"file",revision:"1"}});
 const wire=serializeAgentSourceReferences(draft,{kind:"manuscript",resourceId:"paper",conversationId:"chat"});
 const {host}=await mount(<div className="writing-agent-transcript"><article className="writing-agent-message user"><div className="writing-agent-message-bubble"><AgentUserMessage message={{id:"user",role:"user",content:wire.text,userInput:wire.userInput}}/></div></article><article className="writing-agent-message assistant"><div className="writing-agent-message-bubble"><AgentMarkdown value={String.raw`回答 \(x_i\)。`}/></div></article></div>);
 const start=host.querySelector('.agent-user-message-text')!,end=host.querySelector('.writing-agent-markdown')!,range=document.createRange();
 range.setStartBefore(start);range.setEndAfter(end);
 const {data}=copy(range),restored=readComposerClipboard(data)!;
 expect(restored.bindings).toEqual(draft.bindings);expect(restored.text).toContain("\\(x_i\\)");
 expect(data.getData("text/html")).toContain("<math");expect(data.getData("text/html")).not.toContain("katex-html");
});
it("leaves unrelated copy, collapsed selections and clipboard fallback textareas alone",async()=>{
 const {host}=await mount(<><AgentMarkdown value="**answer**"/><textarea defaultValue="input"/></>);
 const range=document.createRange();range.selectNodeContents(host.querySelector("textarea")!);
 expect(copy(range).event.defaultPrevented).toBe(false);
 const body=host.querySelector('.writing-agent-markdown')!;range.selectNodeContents(body);range.collapse(true);
 expect(copy(range).event.defaultPrevented).toBe(false);
 range.selectNodeContents(body);document.getSelection()!.removeAllRanges();document.getSelection()!.addRange(range);
 const inputCopy=new ClipboardEvent("copy",{clipboardData:new DataTransfer(),bubbles:true,cancelable:true});
 host.querySelector("textarea")!.dispatchEvent(inputCopy);
 expect(inputCopy.defaultPrevented).toBe(false);
});
it("combines assistant selections in document order and keeps the live selection unchanged",async()=>{
 const {host}=await mount(<div className="writing-agent-transcript"><article className="writing-agent-message"><AgentMarkdown value={String.raw`**first** \(x\)`}/></article><article className="writing-agent-message"><AgentMarkdown value={String.raw`second \(y\)`}/></article></div>);
 const bodies=host.querySelectorAll('.writing-agent-markdown'),range=document.createRange();
 range.selectNodeContents(bodies[0]!);range.setEnd(bodies[1]!,bodies[1]!.childNodes.length);
 const {data}=copy(range);
 expect(data.getData("text/plain")).toBe("**first** \\(x\\)\n\nsecond \\(y\\)");
 expect(document.getSelection()!.getRangeAt(0).startContainer).toBe(range.startContainer);
 const html=document.createElement("div");html.innerHTML=data.getData("text/html");expect(html.querySelectorAll("math")).toHaveLength(2);
});

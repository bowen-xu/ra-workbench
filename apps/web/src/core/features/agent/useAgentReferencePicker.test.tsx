/** @vitest-environment happy-dom */
import {act, useRef, useState} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it} from "vitest";
import {EditorView} from "@codemirror/view";
import {deleteCharBackward, deleteCharForward, undo, redo} from "@codemirror/commands";
import {useAgentComposerInput} from "./useAgentComposerInput";
import {AgentComposerTextInput, type AgentComposerInputHandle} from "./AgentComposerTextInput";
import {AgentResourcePickerMenu} from "./AgentResourcePickerMenu";
import {createMockAgentResourceGateways} from "../../../app/previews/agent/mockAgentResourceGateways";
import type {AgentResourceGateways} from "./agentResourceGateways";
import {composerEditorDraft} from "./agentComposerEditor";
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
afterEach(()=>document.body.replaceChildren());
const resources:AgentResourceGateways={...createMockAgentResourceGateways(),tools: {...createMockAgentResourceGateways().tools, list: async()=>[{serverId:"模拟文献检索",environmentId:"mock",revision:"1",enabled:true,credentialConfigured:true,approved:true}]},hostResources:async()=>[{group:"files",label:"main.tex",source:"当前稿件",input:{type:"mention",name:"main.tex",target:{kind:"file",sourceId:"main.tex",resourceId:"fixture",revision:"file-v1"}}},{group:"skills",label:"模拟写作技能",source:"当前稿件",input:{type:"skill",name:"模拟写作技能",identity:{environment_id:"manuscript:fixture",source_id:"local/SKILL.md",revision:"2"}}}]};
const scope={conversationId:crypto.randomUUID(),kind:"manuscript" as const,resourceId:"fixture"};
function Harness(){const [text,setText]=useState("");const input=useRef<AgentComposerInputHandle>(null);const composer=useAgentComposerInput({text,setText,input,gateways:resources,scope});return <><AgentComposerTextInput inputRef={input} aria-label="消息" draft={composer.draft} onDraftChange={composer.changeDraft} onChange={composer.change} onCaretChange={composer.update} onKeyDown={event=>composer.keyDown(event)} gateways={resources}/>{composer.menu}<AgentResourcePickerMenu gateways={resources} onSelect={composer.select}/><button onClick={()=>composer.change("")}>清空已发送草稿</button><output>{JSON.stringify(composer.userInput())}</output></>;}
async function type(input:HTMLTextAreaElement,value:string){await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,value);input.setSelectionRange(value.length,value.length);input.dispatchEvent(new Event("input",{bubbles:true}));});}
function editor(host:HTMLElement){return EditorView.findFromDOM(host.querySelector<HTMLElement>('.agent-composer-inline-input .cm-editor')!)!;}
async function mount(){const host=document.createElement("div");document.body.append(host);const root=createRoot(host);await act(async()=>root.render(<Harness/>));return {host,close:()=>act(async()=>root.unmount())};}
it("Tab inserts the selected opened file instead of entering search",async()=>{
 const view=await mount();try {
 await type(view.host.querySelector("textarea")!,"@main.tex");
 await act(async()=>view.host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",bubbles:true,cancelable:true})));
 expect(document.body.querySelector('[aria-label="选择 Agent 资源"]')).toBeNull();
 expect(composerEditorDraft(editor(view.host).state).bindings[0]?.input).toMatchObject({type:"mention",target:{kind:"file",sourceId:"main.tex"}});
 }finally{await view.close();}
});
it("Tab enters full-library search only when Search is selected, then inserts its selected result",async()=>{
 const view=await mount();try {
 const input=view.host.querySelector("textarea")!;await type(input,"@");
 expect(document.body.querySelector('[aria-label="搜索"]')?.textContent).not.toContain("Tab 查看更多");
 await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"ArrowDown",bubbles:true,cancelable:true})));
 expect(document.body.querySelector('[aria-label="搜索"]')?.classList.contains("is-selected")).toBe(true);
 await act(async()=>input.dispatchEvent(new KeyboardEvent("keyup",{key:"ArrowDown",bubbles:true,cancelable:true})));
 expect(document.body.querySelector('[aria-label="搜索"]')?.textContent).toContain("Tab 查看更多");
 await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",bubbles:true,cancelable:true})));
 expect(document.body.querySelector('[aria-label="返回已打开的文件"]')).not.toBeNull();
 expect(view.host.querySelector("output")?.textContent).toBe("[]");
 await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",bubbles:true,cancelable:true})));
 expect(composerEditorDraft(editor(view.host).state).bindings[0]?.input).toMatchObject({target:{sourceId:"main.tex"}});
 }finally{await view.close();}
});
it.each(["模拟文献检索","模拟写作技能"])("Tab confirms the focused %s menu item",async name=>{
 const view=await mount();try {
 await type(view.host.querySelector("textarea")!,"@"+name);
 const item=document.body.querySelector<HTMLButtonElement>('.agent-resource-item.is-selected')!;
 await act(async()=>{item.focus();item.dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",bubbles:true,cancelable:true}));});
 expect(document.body.querySelector('[aria-label="选择 Agent 资源"]')).toBeNull();
 expect(composerEditorDraft(editor(view.host).state).bindings[0]?.input.name).toBe(name);
 }finally{await view.close();}
});
it("pointer selection replaces Search before Tab confirms the selected tool",async()=>{
 const view=await mount();try {
 const input=view.host.querySelector("textarea")!;await type(input,"@");
 await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"ArrowDown",bubbles:true,cancelable:true})));
 await act(async()=>input.dispatchEvent(new KeyboardEvent("keyup",{key:"ArrowDown",bubbles:true,cancelable:true})));
 expect(document.body.querySelector('[aria-label="搜索"]')?.textContent).toContain("Tab 查看更多");
 const item=document.body.querySelector<HTMLButtonElement>('.agent-resource-group-tools button')!;
 await act(async()=>item.dispatchEvent(new PointerEvent("pointermove",{bubbles:true})));
 expect(item.classList.contains("is-selected")).toBe(true);
 expect(document.body.querySelector('[aria-label="搜索"]')?.textContent).not.toContain("Tab 查看更多");
 await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",bubbles:true,cancelable:true})));
 expect(composerEditorDraft(editor(view.host).state).bindings[0]?.input).toMatchObject({type:"mention",target:{kind:"tool",serverId:"模拟文献检索"}});
 expect(document.body.querySelector('[aria-label="返回已打开的文件"]')).toBeNull();
 }finally{await view.close();}
});
it("Tab also inserts the selected skill using its exact catalog identity",async()=>{
 const view=await mount();try {
 await type(view.host.querySelector("textarea")!,"@模拟写作技能");
 await act(async()=>view.host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",bubbles:true,cancelable:true})));
 expect(composerEditorDraft(editor(view.host).state).bindings[0]?.input).toMatchObject({type:"skill",identity:{source_id:"mock/academic-writing/SKILL.md"}});
 }finally{await view.close();}
});
it.each(["@"])("%s inserts a same-name skill inline at the caret, retaining its exact source",async trigger=>{
 const view=await mount();try {await type(view.host.querySelector("textarea")!,"前文 "+trigger+"模拟写作技能");expect(document.body.textContent).toContain("当前稿件");const input=view.host.querySelector("textarea")!;
 await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"ArrowDown",bubbles:true,cancelable:true})));
 await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true})));
 expect(document.body.querySelector('[aria-label="选择 Agent 资源"]')).toBeNull();
 const cm=editor(view.host);expect(cm.state.doc.toString()).toBe("前文 @模拟写作技能 ");
 expect(view.host.querySelector("output")?.textContent).toContain("manuscript:fixture");
 expect(view.host.querySelector('.cm-line')?.textContent).toBe("前文 模拟写作技能 ");
 expect(view.host.querySelector('.agent-skill-tokens')).toBeNull();expect(view.host.querySelector('.agent-composer-reference button')).toBeNull();
 await act(async()=>cm.dispatch({changes:{from:cm.state.doc.length,insert:"后文"},userEvent:"input.type"}));
 expect(cm.state.doc.toString()).toBe("前文 @模拟写作技能 后文");expect(view.host.querySelector("output")?.textContent).toContain("local/SKILL.md");
 }finally{await view.close();}
});
it("a slash in prose never opens resources or binds a skill",async()=>{
 const view=await mount();try {await type(view.host.querySelector("textarea")!,"前文 /模拟写作技能");
 expect(document.body.querySelector('[aria-label="选择 Agent 资源"]')).toBeNull();
 expect(view.host.querySelector('[aria-label="Agent 命令"]')).toBeNull();
 expect(view.host.querySelector("output")?.textContent).toBe("[]");
 }finally{await view.close();}
});
it("plus selects a bound skill without revealing concrete files",async()=>{
 const view=await mount();try {
 await act(async()=>view.host.querySelector<HTMLButtonElement>('[aria-label="添加上下文和工具"]')!.click());
 expect(view.host.querySelector('input[type="search"]')).toBeNull();
 expect(document.body.querySelector('.agent-resource-group-files')).toBeNull();
 await act(async()=>document.body.querySelector<HTMLButtonElement>('.agent-resource-group-skills button')!.click());
 expect(view.host.querySelector('[aria-label="添加上下文和工具"]')?.getAttribute("aria-expanded")).toBe("false");
 expect(view.host.querySelector("output")?.textContent).toContain("application");
 expect(view.host.querySelector('.cm-content .agent-composer-reference')).not.toBeNull();
 }finally{await view.close();}
});
it("Backspace/Delete remove a whole reference and undo/redo restore its identity, even with no remaining text",async()=>{
 const view=await mount();try {await type(view.host.querySelector("textarea")!,"@模拟写作技能");await act(async()=>view.host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true})));
 const cm=editor(view.host), binding=composerEditorDraft(cm.state).bindings[0]!;
 await act(async()=>{cm.dispatch({selection:{anchor:binding.endUtf16}});expect(deleteCharBackward(cm)).toBe(true);});
 expect(composerEditorDraft(cm.state).bindings).toHaveLength(0);expect(cm.state.doc.toString()).toBe(" ");
 await act(async()=>expect(undo(cm)).toBe(true));expect(composerEditorDraft(cm.state).bindings[0]).toEqual(binding);
 await act(async()=>expect(redo(cm)).toBe(true));expect(composerEditorDraft(cm.state).bindings).toHaveLength(0);
 await act(async()=>undo(cm));await act(async()=>{cm.dispatch({selection:{anchor:binding.startUtf16}});expect(deleteCharForward(cm)).toBe(true);});
 await act(async()=>cm.dispatch({changes:{from:0,to:cm.state.doc.length},userEvent:"delete"}));expect(editor(view.host)).toBe(cm);
 await act(async()=>undo(cm));expect(view.host.querySelector('.agent-composer-reference')).not.toBeNull();
 }finally{await view.close();}
});
it("partial cut/replacement expands to the entire token; Unicode surrounding text and history survive",async()=>{
 const view=await mount();try {await type(view.host.querySelector("textarea")!,"😀 @模拟写作技能");await act(async()=>view.host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true})));
 const cm=editor(view.host);const binding=composerEditorDraft(cm.state).bindings[0]!;
 await act(async()=>cm.dispatch({changes:{from:binding.startUtf16+1,to:binding.endUtf16-1,insert:"替换"},userEvent:"input.paste"}));expect(cm.state.doc.toString()).toBe("😀 替换 ");expect(composerEditorDraft(cm.state).bindings).toHaveLength(0);
 await act(async()=>undo(cm));expect(composerEditorDraft(cm.state).bindings[0]).toEqual(binding);
 }finally{await view.close();}
});
it("all @ tool resources have the same atomic inline treatment",async()=>{
 const view=await mount();try {await type(view.host.querySelector("textarea")!,"前文 @模拟文献检索");await act(async()=>view.host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true})));
 const cm=editor(view.host);const binding=composerEditorDraft(cm.state).bindings[0]!;
 expect(binding.input.type).toBe("mention");expect(view.host.querySelector('.cm-content .agent-composer-reference')).not.toBeNull();
 await act(async()=>{cm.dispatch({selection:{anchor:binding.endUtf16}});deleteCharBackward(cm);});expect(composerEditorDraft(cm.state).bindings).toHaveLength(0);expect(cm.state.doc.toString()).toBe("前文  ");
 }finally{await view.close();}
});
it("clearing a submitted draft resets the editor history",async()=>{
 const view=await mount();try {await type(view.host.querySelector("textarea")!,"@模拟写作技能");await act(async()=>view.host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true})));await act(async()=>Array.from(view.host.querySelectorAll('button')).find(button=>button.textContent==='清空已发送草稿')!.click());expect(view.host.querySelector("textarea")?.value).toBe("");expect(view.host.querySelector('.cm-content')).toBeNull();}finally{await view.close();}
});
it("file references selected through @ are inline and atomic too",async()=>{
 const view=await mount();try {await type(view.host.querySelector("textarea")!,"使用 @main.tex");await act(async()=>view.host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true})));
 const cm=editor(view.host), binding=composerEditorDraft(cm.state).bindings[0]!;
 expect(binding.input).toMatchObject({type:"mention",target:{kind:"file",sourceId:"main.tex",revision:"file-v1"}});
 expect(view.host.querySelector('.cm-line')?.textContent).toBe("使用 main.tex ");
 await act(async()=>{cm.dispatch({selection:{anchor:binding.endUtf16}});deleteCharBackward(cm);});
 expect(composerEditorDraft(cm.state).bindings).toHaveLength(0);
 await act(async()=>undo(cm));expect(composerEditorDraft(cm.state).bindings[0]).toEqual(binding);
 }finally{await view.close();}
});
it("undoing initial selection restores ordinary trigger text without leaving a bound identity",async()=>{
 const view=await mount();try {await type(view.host.querySelector("textarea")!,"前文 @main.tex");await act(async()=>view.host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true})));
 const cm=editor(view.host);await act(async()=>undo(cm));expect(cm.state.doc.toString()).toBe("前文 @main.tex");expect(composerEditorDraft(cm.state).bindings).toHaveLength(0);
 await act(async()=>redo(cm));expect(composerEditorDraft(cm.state).bindings[0]?.input).toMatchObject({target:{kind:"file",revision:"file-v1"}});
 }finally{await view.close();}
});
it.each(["模拟写作技能", "模拟文献检索", "main.tex"])("moving the caret back to bound %s never reopens the resource query",async name=>{
 const view=await mount();try {await type(view.host.querySelector("textarea")!,"前文 @"+name);await act(async()=>view.host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true})));
 const cm=editor(view.host), binding=composerEditorDraft(cm.state).bindings[0]!;
 for(const position of [binding.endUtf16, binding.startUtf16+1, binding.startUtf16, binding.endUtf16]) {
   await act(async()=>cm.dispatch({selection:{anchor:position}}));
   expect(document.body.querySelector('[aria-label="选择 Agent 资源"]')).toBeNull();
 }
 expect(composerEditorDraft(cm.state).bindings[0]).toEqual(binding);
 await act(async()=>cm.dispatch({changes:{from:cm.state.doc.length,insert:"@main"},selection:{anchor:cm.state.doc.length+5},userEvent:"input.type"}));
 expect(document.body.querySelector('[aria-label="选择 Agent 资源"]')).not.toBeNull();
 expect(composerEditorDraft(cm.state).bindings[0]).toEqual(binding);
 }finally{await view.close();}
});

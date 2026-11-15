/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {AgentCanvasDemo} from "./AgentCanvasDemo";
import {enqueueCanvasMessage} from "./AgentCanvasDemo.testSupport";
import { selectAppStorage } from "../../../core/runtime/appStorage";
// Lifecycle tests keep mock emission fast; dedicated pacing tests cover defaults.
vi.mock("./WritingDemoTextStream", async importOriginal => ({
  ...await importOriginal<typeof import("./WritingDemoTextStream")>(),
  DEFAULT_DEMO_TEXT_PACING: { charactersPerChunk: 1000, intervalMs: 10 },
}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
beforeEach(() => {
 const values = new Map<string, string>();
 selectAppStorage({get length(){return values.size;},key:index=>[...values.keys()][index]??null,
  getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},
  removeItem:key=>{values.delete(key);},clear:()=>values.clear()});
});
afterEach(()=>{document.body.innerHTML="";});
async function mount(){const container=document.createElement("div");document.body.append(container);const root=createRoot(container);await act(async()=>root.render(<AgentCanvasDemo/>));await vi.waitFor(async()=>{await act(async()=>new Promise(resolve=>setTimeout(resolve,10)));expect(container.querySelector('[data-action="stop"]')).not.toBeNull();},{timeout:4000});return {container,close:async()=>{await act(async()=>root.unmount());container.remove();}};}
function type(textarea:HTMLTextAreaElement,value:string){Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")?.set?.call(textarea,value);textarea.dispatchEvent(new Event("input",{bubbles:true}));}
it("streams rich Markdown messages through the production renderer",async()=>{
 const view=await mount();try{
  vi.useFakeTimers();
  await act(async()=>{const scenario=view.container.querySelector<HTMLSelectElement>('[aria-label="模拟消息场景"]')!;scenario.value="markdown";scenario.dispatchEvent(new Event("change",{bubbles:true}));});
  await vi.waitFor(async()=>{await act(async()=>Promise.resolve());
   expect(view.container.querySelector('[aria-label="模拟消息场景"]')).toHaveProperty("value","markdown");
   expect(view.container.querySelector<HTMLTextAreaElement>('textarea')?.disabled).toBe(false);
   expect(view.container.querySelector('.writing-agent-transcript h1')?.textContent).toBe("稿件检查");
  },{timeout:4000});
  for(let step=0;step<240;step++){
   await act(async()=>vi.advanceTimersByTimeAsync(300));
   if(view.container.querySelector('.writing-agent-transcript')?.textContent?.includes("模拟检查完成。"))break;
  }
  const messages=view.container.querySelector('.writing-agent-transcript')!;
  expect(messages.querySelector('h1')?.textContent).toBe("稿件检查");
  expect(messages.querySelector('h2')?.textContent).toBe("修改对照");
  expect(messages.querySelector('h3')?.textContent).toBe("检查步骤");
  expect(messages.querySelector('strong')?.textContent).toBe("模拟反馈");
  expect(messages.querySelector('del')?.textContent).toBe("删除线");
  expect(messages.querySelector('blockquote')?.textContent).toContain("演示数据");
  expect(messages.querySelectorAll('table tbody tr')).toHaveLength(2);
  expect(messages.querySelectorAll('.katex-display')).toHaveLength(4);
  expect(messages.querySelectorAll('.katex')).toHaveLength(8);
  expect(messages.querySelector('.katex-error')).toBeNull();
  expect(messages.querySelector('pre code')?.textContent).toContain("mean = sum(values) / len(values)");
  expect(messages.querySelectorAll('input[type="checkbox"]:checked')).toHaveLength(2);
  expect(messages.querySelector('a')?.getAttribute('href')).toBe("https://commonmark.org/help/");
 }finally{await view.close();vi.useRealTimers();}
},10000);

it("starts an unsaved new conversation and can reopen the previous background progress and draft",async()=>{
 const view=await mount();try{
  const duration=view.container.querySelector<HTMLSelectElement>('[aria-label="模拟运行时长"]')!;
  await act(async()=>{duration.value=String(1/60);duration.dispatchEvent(new Event("change",{bubbles:true}));});
  await act(async()=>type(view.container.querySelector<HTMLTextAreaElement>('textarea')!,"保留原对话草稿"));
  await act(async()=>view.container.querySelector<HTMLButtonElement>('[aria-label="新建会话"]')!.click());
  expect(view.container.querySelector('.writing-agent-panel')).not.toBeNull();
  expect(view.container.querySelector('.writing-agent-header-subtitle')?.textContent).toBe("新对话");
  expect(view.container.querySelector('[aria-label="关闭当前对话"]')).toBeNull();
  expect(view.container.querySelector<HTMLTextAreaElement>('textarea')?.value).toBe("");
  expect(view.container.querySelector('.writing-agent-transcript')?.textContent).not.toContain("开始处理当前稿件");
  await act(async()=>view.container.querySelector<HTMLButtonElement>('[aria-label="新建会话"]')!.click());
  await act(async()=>view.container.querySelector<HTMLButtonElement>('[aria-label="对话列表"]')!.click());
  await vi.waitFor(()=>expect(view.container.querySelectorAll('.conversation-list-row')).toHaveLength(2));
  const original=[...view.container.querySelectorAll<HTMLButtonElement>('.conversation-list-row > button')].find(button=>button.textContent==="完善摘要")!;
  await act(async()=>original.click());
  expect(view.container.querySelector<HTMLTextAreaElement>('textarea')?.value).toBe("保留原对话草稿");
  await vi.waitFor(async()=>{await act(async()=>Promise.resolve());expect(view.container.querySelector('.writing-agent-transcript')?.textContent).toContain("已完成");},{timeout:4000});
  expect(view.container.querySelector('.writing-agent-transcript')?.textContent).not.toContain("已中止");
 }finally{await view.close();}
},10000);
it("keeps production input editable during a run and submits through the real queue controller",async()=>{
 const view=await mount();try{expect(view.container.querySelector('[aria-label="队列消息"]')).toBeNull();expect(view.container.querySelector('.writing-agent-goal')).toBeNull();const add=view.container.querySelector<HTMLButtonElement>('[aria-label="添加上下文和工具"]')!;expect(add.disabled).toBe(false);const textarea=view.container.querySelector<HTMLTextAreaElement>('textarea')!;await act(async()=>type(textarea,"后续检查图表"));const send=view.container.querySelector<HTMLButtonElement>('[data-action="send"]')!;expect(send.disabled).toBe(false);await act(async()=>send.click());await vi.waitFor(async()=>{await act(async()=>Promise.resolve());expect(view.container.querySelector('.agent-input-queue-body')?.textContent).toContain("后续检查图表");expect(textarea.value).toBe("");});expect(view.container.querySelector('[data-agent-activity="thinking"]')).not.toBeNull();}finally{await view.close();}
});
it("retains pending Steer until formal history confirms it",async()=>{
 const view=await mount();try{
  await enqueueCanvasMessage(view.container,"修改 keywords 部分");
  await enqueueCanvasMessage(view.container,"按百分比报告进度");
  vi.useFakeTimers();
  const buttons=[...view.container.querySelectorAll<HTMLButtonElement>('button[aria-label$="条消息插入当前轮次"]')];
  expect(buttons).toHaveLength(2);await act(async()=>buttons[0]?.click());
  expect(view.container.querySelector('[data-pending-steering]')?.textContent).toContain("修改 keywords 部分");
  expect(view.container.querySelector('.agent-input-queue-body')?.textContent ?? "").not.toContain("修改 keywords 部分");
  expect(view.container.querySelector('.agent-input-queue-body')?.textContent).toContain("按百分比报告进度");
  await act(async()=>{const duration=view.container.querySelector<HTMLSelectElement>('[aria-label="模拟运行时长"]')!;duration.value=String(1/60);duration.dispatchEvent(new Event("change",{bubbles:true}));});
  await act(async()=>vi.advanceTimersByTimeAsync(1300));
  expect(view.container.querySelector('.writing-agent-transcript')?.textContent).toContain("修改 keywords 部分");
  vi.useRealTimers();
  await vi.waitFor(async()=>{
   await act(async()=>Promise.resolve());
   expect(view.container.querySelector('[data-pending-steering]')).toBeNull();
   expect([...view.container.querySelectorAll('.writing-agent-message.user p')].some(item=>item.textContent==="修改 keywords 部分")).toBe(true);
   expect(view.container.querySelector('.agent-input-queue-body')?.textContent ?? "").not.toContain("修改 keywords 部分");
  },{timeout:4000});
 }finally{await view.close();vi.useRealTimers();}
});
it("uses the shared selector and complete tool/failure lifecycle without legacy panels",async()=>{
 const view=await mount();try{expect(view.container.textContent).not.toContain("正在生成");expect(view.container.textContent).not.toContain("查看本次副本变更");const scenario=view.container.querySelector<HTMLSelectElement>('[aria-label="模拟消息场景"]')!;await act(async()=>{scenario.value="resources";scenario.dispatchEvent(new Event("change",{bubbles:true}));});await act(async()=>{const duration=view.container.querySelector<HTMLSelectElement>('[aria-label="模拟运行时长"]')!;duration.value=String(1/60);duration.dispatchEvent(new Event("change",{bubbles:true}));});await vi.waitFor(async()=>{await act(async()=>new Promise(resolve=>setTimeout(resolve,10)));expect(view.container.querySelector('[data-agent-activity-group]')?.textContent).toContain("加载工具");},{timeout:4000});
 expect(view.container.querySelector('[data-agent-activity-group] [data-tool-status="success"]')).not.toBeNull();
 expect(view.container.querySelector('[data-agent-activity-group]')?.textContent).toContain("模拟写作技能");
 expect([...view.container.querySelectorAll('.writing-agent-message.assistant')].some(item=>item.textContent?.includes("已加载技能"))).toBe(false);
 expect(view.container.querySelector('[data-agent-activity-group] .writing-agent-message-meta')).toBeNull();
 await act(async()=>{const current=view.container.querySelector<HTMLSelectElement>('[aria-label="模拟消息场景"]')!;current.value="failed";current.dispatchEvent(new Event("change",{bubbles:true}));});await vi.waitFor(async()=>{await act(async()=>new Promise(resolve=>setTimeout(resolve,10)));expect(view.container.querySelector('.writing-agent-transcript')?.textContent).toContain("依赖缺失");},{timeout:4000});}finally{await view.close();}
});
it("uses one scenario selector and starts the multiple-response fixture without duplicate buttons",async()=>{
 const view=await mount();try{
  expect(view.container.querySelector('.agent-canvas-demo-notice')).toBeNull();
  expect(view.container.textContent).not.toContain("当前对话正在思考，队列消息会持续保留。");
  expect(view.container.querySelectorAll('[aria-label="模拟消息场景"]')).toHaveLength(1);
  expect([...view.container.querySelectorAll('footer button')].map(button=>button.textContent)).toEqual(["重置假数据", "触发队列错误", "模拟网络中断"]);
  await act(async()=>{const scenario=view.container.querySelector<HTMLSelectElement>('[aria-label="模拟消息场景"]')!;scenario.value="multiple";scenario.dispatchEvent(new Event("change",{bubbles:true}));});
  await vi.waitFor(async()=>{
   await act(async()=>Promise.resolve());
   expect(view.container.querySelector('[aria-label="模拟消息场景"]')).toHaveProperty("value","multiple");
   expect(view.container.querySelector('[aria-label="模拟运行时长"]')).toHaveProperty("value","0.05");
   expect(view.container.querySelector('.writing-agent-transcript')?.textContent).toContain("已完成第一次修改");
  },{timeout:8000});
  expect(view.container.querySelector('[data-action="stop"]')).not.toBeNull();
  expect(view.container.querySelector('[data-agent-activity-group], [data-agent-activity-single]')).not.toBeNull();
 }finally{await view.close();}
},10000);
it("auto-starts the next queued turn after three seconds with editor identity and keeps Steer available",async()=>{
 const view=await mount();try{
  await enqueueCanvasMessage(view.container,"修改 keywords 部分");
  await enqueueCanvasMessage(view.container,"按百分比报告进度");
  const duration=view.container.querySelector<HTMLSelectElement>('[aria-label="模拟运行时长"]')!;
  expect([...duration.options].map(option=>option.text)).toContain("1 秒");
  await act(async()=>{duration.value="0.05";duration.dispatchEvent(new Event("change",{bubbles:true}));});
  await vi.waitFor(async()=>{await act(async()=>Promise.resolve());
   expect(view.container.querySelector('.writing-agent-transcript')?.textContent).toContain("修改 keywords 部分");
   expect(view.container.querySelector('.agent-input-queue-body')?.textContent).not.toContain("修改 keywords 部分");
   expect(view.container.querySelector('button[aria-label="将第 1 条消息插入当前轮次"]')).not.toBeNull();
  },{timeout:5000});
  expect(document.body.textContent).not.toContain("编辑器内容正在切换");
  await act(async()=>view.container.querySelector<HTMLButtonElement>('button[aria-label="将第 1 条消息插入当前轮次"]')!.click());
  await vi.waitFor(async()=>{await act(async()=>Promise.resolve());expect(view.container.querySelector('.writing-agent-transcript')?.textContent).toContain("按百分比报告进度");});
 }finally{await view.close();}
});

it("opens the production conversation list from the header and preserves its search while hidden",async()=>{
 const view=await mount();try{
  expect(view.container.querySelector('[aria-label="对话列表面板"]')).toBeNull();
  const panel=view.container.querySelector('.writing-agent-panel');
  expect(panel?.closest('.agent-canvas-demo-surface')).not.toBeNull();
  expect(panel?.closest('.agent-canvas-shell')?.getAttribute('data-placement')).toBe("contained");
  const toggle=view.container.querySelector<HTMLButtonElement>('button[aria-label="对话列表"]')!;
  await act(async()=>toggle.click());
  await vi.waitFor(async()=>{await act(async()=>Promise.resolve());expect(view.container.querySelector('.conversation-list-row')?.textContent).toContain("第二个对话");});
  expect(view.container.querySelector('input[aria-label="搜索对话标题或消息"]')).toBeNull();
  await act(async()=>view.container.querySelector<HTMLButtonElement>('[aria-label="搜索对话"]')!.click());
  let input=view.container.querySelector<HTMLInputElement>('input[aria-label="搜索对话标题或消息"]')!;
  const setter=Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype,"value")!.set!;
  await act(async()=>{setter.call(input,"摘要");input.dispatchEvent(new Event("input",{bubbles:true}));});
  await act(async()=>input.dispatchEvent(new window.KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));
  expect(view.container.querySelector<HTMLElement>('[aria-label="对话列表面板"]')!.hidden).toBe(false);
  // Escape closes and clears search; it does not hide either panel.
  expect(view.container.querySelector('input[aria-label="搜索对话标题或消息"]')).toBeNull();
  await act(async()=>view.container.querySelector<HTMLButtonElement>('[aria-label="搜索对话"]')!.click());
  input=view.container.querySelector<HTMLInputElement>('input[aria-label="搜索对话标题或消息"]')!;
  expect(input.value).toBe("");
  await act(async()=>{setter.call(input,"摘要");input.dispatchEvent(new Event("input",{bubbles:true}));});
  await act(async()=>view.container.querySelector<HTMLButtonElement>('[aria-label="隐藏对话列表"]')!.click());
  expect(view.container.querySelector<HTMLElement>('[aria-label="对话列表面板"]')!.hidden).toBe(true);
  expect(view.container.querySelector('.writing-agent-panel')).toBe(panel);
  expect(view.container.querySelector('[data-action="stop"]')).not.toBeNull();
  expect(document.activeElement).toBe(toggle);
  await act(async()=>toggle.click());
  expect(view.container.querySelector<HTMLInputElement>('input[aria-label="搜索对话标题或消息"]')!.value).toBe("摘要");
 }finally{await view.close();}
});

it("labels a pending conversation without persisting it until the first send",async()=>{
 const view=await mount();try{
  const title=()=>view.container.querySelector('.writing-agent-header-subtitle')?.textContent;
  await vi.waitFor(()=>expect(title()).toBe("完善摘要"));
  await act(async()=>view.container.querySelector<HTMLButtonElement>('[aria-label="对话列表"]')!.click());
  await vi.waitFor(()=>expect(view.container.querySelector('[aria-label="新对话"]')).not.toBeNull());
  await act(async()=>view.container.querySelector<HTMLButtonElement>('[aria-label="新建会话"]')!.click());
  expect(title()).toBe("新对话");
  await act(async()=>type(view.container.querySelector<HTMLTextAreaElement>('textarea')!,"讨论实验设计"));
  expect(title()).toBe("新对话");
  expect(view.container.querySelector<HTMLButtonElement>('[data-action="send"]')?.disabled).toBe(false);
  await act(async()=>view.container.querySelector('form.writing-agent-composer')!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true})));
  await vi.waitFor(async()=>{await act(async()=>Promise.resolve());expect(title()).toBe("讨论实验设计");},{timeout:5000});
 }finally{await view.close();}
},10000);

it("reveals an opened document by collapsing chat without stopping or discarding the conversation",async()=>{
 const {openTextDocument}=await import("../../../core/components/text-editor/textDocumentEvents");
 const view=await mount();try{
  await act(async()=>{expect(openTextDocument({id:"reference:test",path:"SKILL.md",content:"# Reference",readOnly:true})).toBe(true);});
  const launcher=view.container.querySelector<HTMLButtonElement>('[aria-label="打开对话"]');
  expect(launcher).not.toBeNull();expect(launcher!.getAttribute("aria-busy")).toBe("true");
  expect(view.container.querySelector('.text-editor')).not.toBeNull();
  await act(async()=>launcher!.click());
  expect(view.container.querySelector('[data-action="stop"]')).not.toBeNull();
  expect(view.container.querySelector('.writing-agent-transcript')?.textContent).toContain("开始处理当前稿件");
 }finally{await view.close();}
});

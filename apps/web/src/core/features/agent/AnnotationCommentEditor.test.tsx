/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { AnnotationCommentEditor } from "./AnnotationCommentEditor";
import { SpeechInputAdapterProvider } from "../speech/SpeechInputAdapter";
import { writingDemoSpeechAdapter } from "../../../app/previews/agent/WritingDemoSpeechAdapter";
import { notify } from "../../services/notifications";
vi.mock("../../services/notifications",()=>({notify:{error:vi.fn()}}));
const cleanup:(()=>Promise<void>)[]=[];
afterEach(async()=>{for(const close of cleanup.splice(0))await close();vi.unstubAllGlobals();vi.clearAllMocks();});
async function mount(props:React.ComponentProps<typeof AnnotationCommentEditor>){
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const host=document.createElement("div"),root=createRoot(host);document.body.append(host);
  await act(async()=>root.render(<SpeechInputAdapterProvider adapter={writingDemoSpeechAdapter}><AnnotationCommentEditor {...props}/></SpeechInputAdapterProvider>));
  cleanup.push(async()=>{await act(async()=>root.unmount());host.remove();});
  return host;
}
async function type(host:HTMLElement,value:string){await act(async()=>{const input=host.querySelector("textarea")!;Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,value);input.dispatchEvent(new Event("input",{bubbles:true}));});}
it("shows the shared microphone initially, a check after typing, and Send only while the modifier is held",async()=>{
  const add=vi.fn(),send=vi.fn(),cancel=vi.fn();const host=await mount({title:"原文",mode:"create",onSave:add,onSend:send,onCancel:cancel});
  expect(host.querySelector('[aria-label="语音输入"]')).not.toBeNull();expect(host.querySelector(".annotation-comment-commit")).toBeNull();
  await type(host,"问题");expect(host.querySelector('[aria-label="添加批注"] .lucide-check')).not.toBeNull();
  await act(async()=>window.dispatchEvent(new KeyboardEvent("keydown",{key:"Meta",metaKey:true})));
  expect(host.querySelector('[aria-label="发送批注问题"] .lucide-arrow-up')).not.toBeNull();
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="发送批注问题"]')!.click());expect(send).toHaveBeenCalledExactlyOnceWith("问题");expect(add).not.toHaveBeenCalled();
  await act(async()=>window.dispatchEvent(new Event("blur")));expect(host.querySelector('[aria-label="添加批注"]')).not.toBeNull();
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="添加批注"]')!.click());expect(add).toHaveBeenCalledExactlyOnceWith("问题");
});
it("allows empty Add, suppresses IME submission, sends Cmd+Enter and retains text after a failed send",async()=>{
  const add=vi.fn(),send=vi.fn().mockRejectedValue(new Error("模型忙，请重试"));const host=await mount({title:"原文",mode:"create",onSave:add,onSend:send,onCancel:vi.fn()});
  const key=async(options:KeyboardEventInit)=>act(async()=>host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true,...options})));
  await key({isComposing:true});expect(add).not.toHaveBeenCalled();await key({});expect(add).toHaveBeenCalledWith("");
  await type(host,"问题");await key({metaKey:true});expect(send).toHaveBeenCalledExactlyOnceWith("问题");
  expect(notify.error).toHaveBeenCalledWith("模型忙，请重试");expect(host.querySelector("textarea")?.value).toBe("问题");
});
it("uses a separate editing layout with delete, shared microphone, Cancel and Save",async()=>{
  const add=vi.fn(),cancel=vi.fn(),remove=vi.fn();const host=await mount({title:"原文",initialComment:"评论",onSave:add,onCancel:cancel,onRemove:remove});
  expect(host.querySelector(".is-create")).toBeNull();expect(host.querySelector('[aria-label="语音输入"]')).not.toBeNull();expect(host.querySelector(".annotation-comment-commit")).toBeNull();
  await type(host,"修改");await act(async()=>[...host.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent==="取消")!.click());expect(cancel).toHaveBeenCalledOnce();expect(add).not.toHaveBeenCalled();
});
it("uses the shared multiline layout for existing comments and shrinks after editing back to one line",async()=>{
  const text="第一行\n第二行\n第三行", save=vi.fn();
  const host=await mount({title:"原文",initialComment:text,onSave:save,onCancel:vi.fn()});
  expect(host.querySelector("textarea")?.value).toBe(text);
  expect(host.querySelector(".is-multiline")).not.toBeNull();
  await type(host,"单行");expect(host.querySelector(".is-multiline")).toBeNull();
  await act(async()=>host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",metaKey:true,bubbles:true,cancelable:true})));
  expect(save).toHaveBeenCalledExactlyOnceWith("单行");
});
it.each([{}, {metaKey:true}, {ctrlKey:true}])("saves edited comments on Enter with modifiers %j without sending",async modifiers=>{
  const save=vi.fn(),send=vi.fn();const host=await mount({title:"原文",initialComment:"修改后的评论",onSave:save,onSend:send,onCancel:vi.fn()});
  const input=host.querySelector("textarea")!;
  const key=new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true,...modifiers});
  await act(async()=>input.dispatchEvent(key));
  expect(key.defaultPrevented).toBe(true);expect(save).toHaveBeenCalledExactlyOnceWith("修改后的评论");expect(send).not.toHaveBeenCalled();
});
it("leaves Shift+Enter to newline entry and does not save while composing in the edit form",async()=>{
  const save=vi.fn();const host=await mount({title:"原文",initialComment:"评论",onSave:save,onCancel:vi.fn()});
  const input=host.querySelector("textarea")!;
  for(const modifiers of [{shiftKey:true},{isComposing:true},{keyCode:229}]){
    const key=new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true,...modifiers});
    await act(async()=>input.dispatchEvent(key));expect(key.defaultPrevented).toBe(false);
  }
  expect(save).not.toHaveBeenCalled();
});
it("keeps speech available alongside the commit button after multiline input",async()=>{
  const host=await mount({title:"原文",mode:"create",onSave:vi.fn(),onSend:vi.fn(),onCancel:vi.fn()});
  await type(host,"第一行\n第二行");
  expect(host.querySelector(".is-multiline")).not.toBeNull();
  expect(host.querySelector('[aria-label="语音输入"]')?.closest("[hidden]")).toBeNull();
  expect(host.querySelector('[aria-label="添加批注"]')).not.toBeNull();
  await type(host,"单行"); expect(host.querySelector(".is-multiline")).toBeNull();
});
it("suppresses scrolling during uncapped expansion, enables it at the cap and clears it after shrinking",async()=>{
  const host=await mount({title:"原文",mode:"create",onSave:vi.fn(),onCancel:vi.fn()});
  const editor=host.querySelector<HTMLElement>(".annotation-comment-editor")!,input=host.querySelector("textarea")!;
  let contentHeight=77, animatedHeight=32, width=295;
  Object.defineProperty(editor,"clientWidth",{get:()=>width});editor.style.padding="6px 10px";
  Object.defineProperty(input,"scrollHeight",{get:()=>contentHeight});
  Object.defineProperty(input,"clientHeight",{get:()=>animatedHeight});
  input.style.lineHeight="22px";input.style.padding="6px 0 6px 10px";
  await type(host,"第一行\n第二行\n第三行");
  expect(input.style.height).toBe("77px");expect(input.style.overflowY).toBe("hidden");
  expect(input.hasAttribute("data-fade-bottom")).toBe(false);
  contentHeight=300;animatedHeight=140;await type(host,Array(12).fill("行").join("\n"));
  expect(input.style.height).toBe("140px");expect(input.style.overflowY).toBe("auto");
  expect(input.hasAttribute("data-fade-bottom")).toBe(true);
  await act(async()=>{input.scrollTop=160;input.dispatchEvent(new Event("scroll"));});
  expect(input.hasAttribute("data-fade-top")).toBe(true);expect(input.hasAttribute("data-fade-bottom")).toBe(false);
  width=0;await type(host,"隐藏时改变正文");
  expect(host.querySelector(".is-multiline")).not.toBeNull();
  width=295;contentHeight=34;await type(host,"单行");
  expect(input.style.overflowY).toBe("hidden");expect(input.hasAttribute("data-fade-top")).toBe(false);
  expect(input.hasAttribute("data-fade-bottom")).toBe(false);
});
it("leaves Tab to normal focus navigation without adding, sending or canceling",async()=>{
  const add=vi.fn(),send=vi.fn(),cancel=vi.fn();
  const host=await mount({title:"原文",mode:"create",onSave:add,onSend:send,onCancel:cancel});
  const key=async(options:KeyboardEventInit)=>act(async()=>host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{bubbles:true,cancelable:true,...options})));
  await key({key:"Tab",shiftKey:true});await key({key:"Tab",isComposing:true});expect(add).not.toHaveBeenCalled();
  await key({key:"Tab"});await type(host,"评论");await key({key:"Tab",metaKey:true});
  expect(add).not.toHaveBeenCalled();expect(send).not.toHaveBeenCalled();expect(cancel).not.toHaveBeenCalled();
});
it.each(["escape","blur","outside"])("silently captures on %s once and keeps internal controls usable",async trigger=>{
  const dismiss=vi.fn(),add=vi.fn(),send=vi.fn(),cancel=vi.fn();
  const host=await mount({title:"原文",mode:"create",onDismiss:dismiss,onSave:add,onSend:send,onCancel:cancel});
  await type(host,"可选评论");
  const input=host.querySelector("textarea")!;
  await act(async()=>input.dispatchEvent(new FocusEvent("focusout",{bubbles:true,relatedTarget:host.querySelector("button")})));
  expect(dismiss).not.toHaveBeenCalled();
  await act(async()=>{
    if(trigger==="escape")input.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true}));
    else if(trigger==="blur")input.dispatchEvent(new FocusEvent("focusout",{bubbles:true,relatedTarget:document.body}));
    else document.body.dispatchEvent(new PointerEvent("pointerdown",{bubbles:true,button:0}));
    input.dispatchEvent(new FocusEvent("focusout",{bubbles:true,relatedTarget:document.body}));
  });
  expect(dismiss).toHaveBeenCalledExactlyOnceWith("可选评论");
  expect(add).not.toHaveBeenCalled();expect(send).not.toHaveBeenCalled();expect(cancel).not.toHaveBeenCalled();
});
it("captures empty quotations silently and allows retry after a failed dismissal",async()=>{
  const dismiss=vi.fn().mockRejectedValueOnce(new Error("保存失败")).mockResolvedValue(undefined);
  const host=await mount({title:"原文",mode:"create",onDismiss:dismiss,onSave:vi.fn(),onCancel:vi.fn()});
  const escape=()=>host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true}));
  await act(async()=>{escape();});expect(notify.error).toHaveBeenCalledWith("保存失败");
  await act(async()=>{escape();});expect(dismiss).toHaveBeenCalledTimes(2);expect(dismiss).toHaveBeenLastCalledWith("");
});

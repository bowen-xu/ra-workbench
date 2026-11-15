/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {AgentInputQueue} from "./AgentInputQueue";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
it("enables steering other rows while edits stay locked, and disables it for a queue error",async()=>{
  const queue={schemaVersion:1 as const,conversationId:"conversation",revision:1,paused:false,preparingInputId:null,
    inputs:[{inputId:"first",text:"first"},{inputId:"second",text:"second"}]};
  const steer=vi.fn(async()=>true),edit=vi.fn(async()=>true),host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const render=(error:string|null)=>root.render(<AgentInputQueue state={{queue,loading:false,editing:false,uncertain:false,error}}
    disabled steerDisabled={false} hiddenInputIds={["first"]} onEdit={edit} onReload={()=>undefined} onRetry={()=>undefined} onSteer={steer}/>);
  try{
    await act(async()=>render(null));
    const button=host.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
    expect(button.disabled).toBe(false);
    expect(host.querySelector<HTMLButtonElement>('[aria-label="移除第 1 条消息"]')?.disabled).toBe(true);
    await act(async()=>button.click());expect(steer).toHaveBeenCalledWith("second","second");expect(edit).not.toHaveBeenCalled();
    await act(async()=>render("读取队列失败，请重试。"));expect(button.disabled).toBe(true);
  }finally{await act(async()=>root.unmount());host.remove();}
});

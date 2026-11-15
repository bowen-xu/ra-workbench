/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {WritingAgentPanel,type WritingAgentMessage} from "./WritingAgentPanel";

vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);

it("hides the welcome state while a provisional submission is visible", async () => {
 const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
 try {
  await act(async () => root.render(<WritingAgentPanel open messages={[]} />));
  expect(host.textContent).toContain("研究助手为您服务");
  await act(async () => root.render(<WritingAgentPanel open messages={[]} hasPendingSubmission submissionControls={<p>临时消息</p>} />));
  expect(host.textContent).toContain("临时消息");
  expect(host.textContent).not.toContain("研究助手为您服务");
  await act(async () => root.render(<WritingAgentPanel open messages={[]} />));
  expect(host.textContent).toContain("研究助手为您服务");
 } finally { await act(async () => root.unmount()); host.remove(); }
});

it("retains the mounted transcript and nested disclosures when hiding and reopening chat",async()=>{
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 const messages:readonly WritingAgentMessage[]=[
  {id:"user",turnId:"turn",role:"user",content:"检查文件",turnFinished:true},
  ...["a","b"].flatMap(id=>[
   {id:`call-${id}`,turnId:"turn",role:"assistant" as const,kind:"tool-call" as const,toolCallId:id,
    toolName:"research_exec_command",content:JSON.stringify({cmd:`cat ${id}.md`}),turnFinished:true},
   {id:`result-${id}`,turnId:"turn",role:"assistant" as const,kind:"tool-result" as const,toolCallId:id,
    toolName:"research_exec_command",content:`${id} output`,turnFinished:true},
  ]),
 ];
 const render=(open:boolean,showLauncher=true)=>act(async()=>root.render(<WritingAgentPanel open={open} showLauncher={showLauncher} messages={messages}/>));
 try{
  await render(false);
  expect(host.querySelector(".writing-agent-panel")).toBeNull();
  await render(true);
  const panel=host.querySelector<HTMLElement>(".writing-agent-panel")!;
  const composer=host.querySelector<HTMLTextAreaElement>("textarea")!;
  expect(document.activeElement).toBe(composer);
  const process=host.querySelector<HTMLDetailsElement>(".writing-agent-turn-process")!;
  await act(async()=>process.querySelector<HTMLElement>(":scope > summary")!.click());
  const group=host.querySelector<HTMLDetailsElement>("[data-agent-activity-group]")!;
  const rows=host.querySelectorAll<HTMLDetailsElement>("details.tool-history-row");
  await act(async()=>{group.open=true;group.dispatchEvent(new Event("toggle"));rows[0]!.open=true;});
  expect(process.open).toBe(true);expect(group.open).toBe(true);expect(rows[0]?.open).toBe(true);
  for(const launcher of [true,false]){
   await render(false,launcher);
   expect(host.querySelector(".writing-agent-panel")).toBe(panel);
   expect(panel.hidden).toBe(true);
   expect(!!host.querySelector(".writing-agent-trigger")).toBe(launcher);
   await render(true,launcher);
   expect(panel.hidden).toBe(false);
   expect(host.querySelector("textarea")).toBe(composer);
   expect(document.activeElement).toBe(composer);
   expect(host.querySelector(".writing-agent-turn-process")).toBe(process);
   expect(host.querySelector("[data-agent-activity-group]")).toBe(group);
   expect(process.open).toBe(true);expect(group.open).toBe(true);
   expect(host.querySelector("details.tool-history-row")).toBe(rows[0]);
   expect(rows[0]?.open).toBe(true);expect(rows[1]?.open).toBe(false);
  }
 }finally{await act(async()=>root.unmount());host.remove();}
});

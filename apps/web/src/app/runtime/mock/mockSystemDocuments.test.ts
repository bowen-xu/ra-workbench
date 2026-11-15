/** @vitest-environment happy-dom */
import {expect,it} from "vitest";
import {createMockSystemDocuments} from "./mockSystemDocuments";
import {mockBundledTaskSkills,readMockBundledTaskSkill} from "./mockBundledTaskSkills";
it("approval is scoped, one-use, and content is active before Keep",async()=>{
  localStorage.clear();const {gateway,requestWrite}=createMockSystemDocuments(localStorage);
  const before=await gateway.read("structured-task"),after=before.content+"\n新增规则。\n";
  const controller=new AbortController();const writing=requestWrite("chat",before.id,after,controller.signal);
  for(let i=0;i<20&&!(await gateway.pending("chat")).length;i++)await new Promise(resolve=>setTimeout(resolve,0));
  const pending=(await gateway.pending("chat"))[0]!;
  expect((await gateway.read(before.id)).content).toBe(before.content);
  await expect(gateway.decide("another-chat",pending.id,true)).rejects.toThrow();
  await gateway.decide("chat",pending.id,true);await writing;
  const saved=await gateway.read(before.id);expect(saved.content).toBe(after);expect(saved.baseline).toBe(before.content);
  await expect(gateway.decide("chat",pending.id,true)).rejects.toThrow();
  await expect(gateway.write({id:before.id,expectedRevision:before.revision,content:after,baseline:after})).rejects.toThrow("文档已变化");
  const reopened=createMockSystemDocuments(localStorage);expect((await reopened.gateway.read(before.id)).content).toBe(after);
  await reopened.gateway.write({id:before.id,expectedRevision:saved.revision,content:saved.baseline,baseline:saved.baseline});
  expect((await gateway.read(before.id)).content).toBe(before.content);
});
it("denial and cancellation leave no write and no reusable approval",async()=>{
  localStorage.clear();const {gateway,requestWrite}=createMockSystemDocuments(localStorage);
  const before=await gateway.read("reading-reference");const abort=new AbortController();
  const writing=requestWrite("chat",before.id,before.content+"\n说明",abort.signal);const rejected=expect(writing).rejects.toThrow("未授权");
  for(let i=0;i<20&&!(await gateway.pending("chat")).length;i++)await new Promise(resolve=>setTimeout(resolve,0));
  await gateway.decide("chat",(await gateway.pending("chat"))[0]!.id,false);await rejected;
  expect((await gateway.read(before.id)).revision).toBe(before.revision);
  const cancelled=requestWrite("chat",before.id,before.content+"\n说明",abort.signal);const rejection=expect(cancelled).rejects.toBeDefined();
  for(let i=0;i<20&&!(await gateway.pending("chat")).length;i++)await new Promise(resolve=>setTimeout(resolve,0));
  abort.abort();await rejection;expect(await gateway.pending("chat")).toEqual([]);
});
it("closed assets retain prompt variables and feed the skill loader immediately",async()=>{
  localStorage.clear();const {gateway}=createMockSystemDocuments(localStorage);
  const before=await gateway.read("generate-paper-portrait");const content=before.content+"\n用户规则。";
  await gateway.write({id:before.id,expectedRevision:before.revision,content,baseline:before.baseline});
  const skill=(await mockBundledTaskSkills(localStorage)).find(skill=>skill.name===before.id)!;
  expect(await readMockBundledTaskSkill({environment_id:"application",source_id:skill.sourceId,revision:skill.revision},localStorage)).toBe(content);
  await expect(gateway.read("../outside.md")).rejects.toThrow();
  const task=await gateway.read("structured-task");
  await expect(gateway.write({id:task.id,expectedRevision:task.revision,content:"missing variables",baseline:task.baseline})).rejects.toThrow();
});

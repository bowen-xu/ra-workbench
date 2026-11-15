import { expect, it } from "vitest";
import { insertAgentReference, type AgentInputDraft } from "@research-assistant/core/agent";
import { ComposerDraftHistory } from "./composerDraftHistory";
const initial: AgentInputDraft={text:"问题",bindings:[],evidence:[{type:"annotation",source:"source://notes/note:#block{b}",title:"引用",text:"原文",comment:"评论"}]};
it("restores exact annotations alongside text and mentions, then discards redo on a new edit",()=>{
  const history=new ComposerDraftHistory(initial);
  const deleted={...initial,evidence:[]};history.record(deleted,1);
  expect(history.undo()).toEqual(initial);expect(history.redo()).toEqual(deleted);
  const skill=insertAgentReference(deleted,{type:"skill",name:"同名技能",identity:{environment_id:"note",source_id:"skills/exact/SKILL.md",revision:"v1"}});
  history.record(skill,2);expect(history.undo()).toEqual(deleted);expect(history.redo()).toEqual(skill);
  history.undo();history.record({...deleted,text:"另一问题"},1000);expect(history.redo()).toBeUndefined();
  expect(history.undo()).toEqual(deleted);expect(history.undo()).toEqual(initial);
});
it("groups contiguous typing, keeps annotation edits separate and isolates submission/scope boundaries",()=>{
  const history=new ComposerDraftHistory(initial);
  history.record({...initial,text:"问题a"},1000);history.record({...initial,text:"问题ab"},1100);
  expect(history.undo()).toEqual(initial);
  history.redo();const comment={...initial,text:"问题ab",evidence:[{...initial.evidence![0]!,comment:"新评论"}]};
  history.record(comment,1200);expect(history.undo()?.evidence).toEqual(initial.evidence);
  history.observe({text:"",bindings:[]});expect(history.undo()).toBeUndefined();expect(history.redo()).toBeUndefined();
  history.record(initial);history.reset({...initial,text:"另一个会话"});expect(history.undo()).toBeUndefined();
});
it("keeps deleted image metadata available in undo and limits history without truncating the current draft",()=>{
  const image={...initial,evidence:[{...initial.evidence![0]!,image:{assetId:"a".repeat(64),captureId:"00000000-0000-4000-8000-000000000001",byteSize:10,mediaType:"image/png" as const}}]};
  const history=new ComposerDraftHistory(image);history.record({...initial,evidence:[]});
  expect(history.retainedDrafts().flatMap(item=>item.evidence??[])).toContainEqual(image.evidence[0]);
  expect(history.undo()).toEqual(image);
  for(let index=0;index<110;index++)history.record({...initial,text:`draft ${index} longer text`},index*1000);
  expect(history.retainedDrafts()).toHaveLength(101);expect(history.undo()?.text).toBe("draft 108 longer text");
});

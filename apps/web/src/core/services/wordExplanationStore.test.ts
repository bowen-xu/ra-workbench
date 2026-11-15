/** @vitest-environment happy-dom */
import { afterEach, expect, it, vi } from "vitest";
import { createBrowserWordExplanationStore } from "./wordExplanationStore";
import type { WordExplanationDraft } from "@research-assistant/core/dictionary";
const memory = vi.hoisted(()=>new Map<string,string>());
vi.mock("../runtime/appStorage",()=>({getAppStorage:()=>({getItem:(key:string)=>memory.get(key)??null,setItem:(key:string,value:string)=>memory.set(key,value)})}));
afterEach(()=>memory.clear());
const draft: WordExplanationDraft = { attachmentId:"pdf",contentSha256:"a".repeat(64),context:{word:"work",before:"In this ",after:", we study vision."},segments:[{pageIndex:0,text:"work",prefix:"In this ",suffix:", we study vision.",rects:[{x:.1,y:.2,width:.1,height:.02}]}],explanation:{translation:"本研究",explanation:"指本文的研究工作。",providerId:"test",modelId:"test"} };
it("reopens stored explanations with occurrence identity and filters changed PDFs",async()=>{
  const first=await createBrowserWordExplanationStore().save(draft);
  const reopened=createBrowserWordExplanationStore();expect(await reopened.list("pdf",draft.contentSha256)).toEqual([first]);
  const updated=await reopened.save({...draft,explanation:{...draft.explanation,translation:"本文工作"}});expect(updated.id).toBe(first.id);
  expect(await reopened.list("pdf",draft.contentSha256)).toEqual([updated]);expect(await reopened.list("pdf","b".repeat(64))).toEqual([]);
  const invalid={...draft,segments:[{...draft.segments[0]!,rects:[{x:.1,y:.2,width:2,height:.02}]}]};
  await expect(reopened.save(invalid)).rejects.toThrow("格式无效");
  expect(await reopened.list("pdf",draft.contentSha256)).toEqual([updated]);
  await reopened.remove(updated); expect(await createBrowserWordExplanationStore().list("pdf",draft.contentSha256)).toEqual([]);
});

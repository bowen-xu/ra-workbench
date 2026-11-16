// @vitest-environment happy-dom
import { expect, it, vi } from "vitest";
import { createMockStructuredResources } from "../../../../../../../../integration/web/src/app/runtime/mock/mockStructuredResources";
import { prepareInitialReadingTemplate } from "../../../../../../../../plugins/ra-literature-library/src/features/ai/resources/prepareInitialReadingTemplate";

async function setup(){
  const values=new Map<string,string>();
  const storage:Storage={get length(){return values.size;},key:index=>[...values.keys()][index]??null,
    getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()};
  const gateway=createMockStructuredResources(storage);
  const template=(await gateway.list("paper")).find(row=>row.kind==="readingTemplate")!;
  const body={kind:"reading_template_v1",schemaVersion:1,name:"本文模板",fields:[{id:"question",title:"问题",question:"要解决的问题"}]};
  await gateway.write({resourceId:template.id,expectedRevision:0,requestId:"suggestion",editorKind:"agent",content:JSON.stringify(body)});
  return {gateway,template};
}

it("applies and adopts the generated paper template on the first extraction action without another confirmation",async()=>{
  const {gateway,template}=await setup();
  const snapshot=await prepareInitialReadingTemplate(gateway,"paper");
  expect(snapshot).toMatchObject({templateResourceId:template.id,templateResourceRevision:1});
  expect((await gateway.get(template.id)).draft).toBeUndefined();
  const reading=(await gateway.list("paper")).find(row=>row.kind==="structuredReading")!;
  expect(reading.templateRef).toEqual({resourceId:template.id,revision:1});
  expect(reading.content).toBeNull();
  const publish=vi.spyOn(gateway,"write");
  await prepareInitialReadingTemplate(gateway,"paper");
  expect(publish).not.toHaveBeenCalled();
});

it("retains invalid or stale drafts without publishing or adopting them",async()=>{
  const {gateway,template}=await setup();
  await gateway.write({resourceId:template.id,expectedRevision:1,requestId:"invalid",editorKind:"agent",content:"{"});
  const publish=vi.spyOn(gateway,"write");
  await expect(prepareInitialReadingTemplate(gateway,"paper")).rejects.toThrow("尚未通过校验");
  expect(publish).not.toHaveBeenCalled();
  expect((await gateway.get(template.id)).draft?.content).toBe("{");
});


it("uses current template content on the next extraction without a version-upgrade UI",async()=>{
  const {gateway,template}=await setup();
  await prepareInitialReadingTemplate(gateway,"paper");
  const current=await gateway.get(template.id);
  await gateway.write({resourceId:template.id,expectedRevision:current.revision,requestId:"later",editorKind:"agent",
    content:JSON.stringify({...current.content as object,name:"新建议"})});
  const publish=vi.spyOn(gateway,"write");
  expect(await prepareInitialReadingTemplate(gateway,"paper")).toMatchObject({templateResourceRevision:2});
  expect(publish).not.toHaveBeenCalled();
  const reading=(await gateway.list("paper")).find(value=>value.kind==="structuredReading")!;
  expect(reading.templateRef).toEqual({resourceId:template.id,revision:2});
  expect((await gateway.get(template.id)).pendingFieldIds.length).toBeGreaterThan(0);
});

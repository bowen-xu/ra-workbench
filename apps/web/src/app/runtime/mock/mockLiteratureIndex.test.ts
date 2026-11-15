// @vitest-environment happy-dom
import {expect,it} from "vitest";
import type {LiteratureGateway} from "@research-assistant/literature-library-plugin/literatureGateway";
import {createMockLiteratureGateway} from "./mockLiteratureGateway";

it("a preview attachment takes the real cold-index path, persists completion and invalidates changed content",async()=>{
  localStorage.clear();
  let hash="synthetic-version-1";
  const paper=()=>({id:"paper",attachments:[{id:"attachment",contentSha256:hash}]});
  const base={list:async()=>[paper()],get:async(id:string)=>{
    if(id!=="paper")throw Error("missing paper");
    return paper();
  }} as unknown as LiteratureGateway;
  const gateway=createMockLiteratureGateway(base,localStorage);
  expect(await gateway.isAttachmentIndexed("attachment")).toBe(false);
  const pages=[{pageIndex:0,text:"Synthetic first page"},{pageIndex:1,text:"Synthetic second page"}];
  await gateway.indexAttachment("paper","attachment",pages);
  expect(await gateway.isAttachmentIndexed("attachment")).toBe(true);
  const reopened=createMockLiteratureGateway(base,localStorage);
  expect(await reopened.isAttachmentIndexed("attachment")).toBe(true);
  expect(JSON.parse(localStorage.getItem("preview-attachment-text-indexes")!)[0][1].pages).toEqual(pages);
  hash="synthetic-version-2";
  expect(await reopened.isAttachmentIndexed("attachment")).toBe(false);
  expect(await reopened.isAttachmentIndexed("missing")).toBe(false);
  await expect(reopened.indexAttachment("paper","missing",pages)).rejects.toThrow("模拟附件不存在");
});

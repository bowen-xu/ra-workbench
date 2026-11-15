import {beforeEach,expect,it,vi} from "vitest";
import {tauriWritingGateway} from "./tauriWritingGateway";
const f = vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock("../../../core/runtime/platformCore",()=>({invoke:f.invoke}));
beforeEach(()=>{f.invoke.mockReset();});
it("queries the writing catalog without enumerating projects/files or reading their bodies",async()=>{
  const result={items:[],revision:3,pendingCount:0,errorCount:0,issues:[]};f.invoke.mockResolvedValue(result);
  const query={category:"all",view:"recent",sort:{field:"title",direction:"descending"},limit:20} as const;
  await expect(tauriWritingGateway.noteCatalog!.queryWritingNoteCatalog(query)).resolves.toEqual(result);
  expect(f.invoke.mock.calls).toEqual([["query_writing_note_catalog",{query}]]);
});
it("keeps a stale source revision and diagnostic code available to the query coordinator",async()=>{
  f.invoke.mockRejectedValue({code:"revision_conflict",message:"note_catalog_changed：目录已更新",currentRevision:19});
  await expect(tauriWritingGateway.noteCatalog!.queryWritingNoteCatalog({category:"all",view:"recent"})).rejects.toMatchObject({code:"revision_conflict",currentRevision:19,message:expect.stringContaining("note_catalog_changed")});
});
it("rejects an accidental canonical body rather than caching it in the list",async()=>{
  f.invoke.mockResolvedValue({items:[],revision:3,pendingCount:0,errorCount:0,issues:[],body:"private"});
  await expect(tauriWritingGateway.noteCatalog!.queryWritingNoteCatalog({category:"all",view:"recent"})).rejects.toThrow("writing_note_catalog_response_invalid");
});
it("prepares one bounded batch only on explicit request and keeps retry intent",async()=>{
  f.invoke.mockResolvedValue({processedCount:8,pendingCount:12,errorCount:1});
  await expect(tauriWritingGateway.noteCatalog!.prepareWritingNoteCatalog()).resolves.toEqual({processedCount:8,pendingCount:12,errorCount:1});
  await tauriWritingGateway.noteCatalog!.prepareWritingNoteCatalog({retryErrors:true});
  await tauriWritingGateway.noteCatalog!.prepareWritingNoteCatalog({scope:"archived"});
  expect(f.invoke.mock.calls).toEqual([["prepare_writing_note_catalog",{retryErrors:false}],["prepare_writing_note_catalog",{retryErrors:true}],["prepare_writing_note_catalog",{retryErrors:false,scope:"archived"}]]);
});

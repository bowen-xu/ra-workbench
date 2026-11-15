import {beforeEach, expect, it, vi} from "vitest";
import {tauriNotePageCatalogGateway} from "./tauriNotesGateway";
const f = vi.hoisted(() => ({invoke:vi.fn()}));
vi.mock("../../../runtime/platformCore", () => ({invoke:f.invoke}));
beforeEach(()=>{f.invoke.mockReset();});
it("dispatches one bounded catalog request without loading any documents", async () => {
  f.invoke.mockResolvedValue({revision:5,items:[]});
  const query = {category:"all",view:"recent",limit:20} as const;
  expect(await tauriNotePageCatalogGateway.queryNotePageCatalog(query)).toEqual({revision:5,items:[]});
  expect(f.invoke.mock.calls).toEqual([["query_note_page_catalog",{query}]]);
});
it("retains backend error codes, catalog diagnostics and known revisions", async () => {
  f.invoke.mockRejectedValue({code:"revision_conflict",message:"note_catalog_changed：目录已更新",currentRevision:19});
  await expect(tauriNotePageCatalogGateway.queryNotePageCatalog({category:"all",view:"recent"})).rejects.toMatchObject({code:"revision_conflict",message:expect.stringContaining("note_catalog_changed"),currentRevision:19});
});
it("rejects an unexpected body in a native catalog response", async () => {
  f.invoke.mockResolvedValue({revision:5,items:[],documents:{private:"body"}});
  await expect(tauriNotePageCatalogGateway.queryNotePageCatalog({category:"all",view:"recent"})).rejects.toThrow("note_catalog_response_invalid");
});
it("dispatches persistent search as one paged request without document hydration",async()=>{
  f.invoke.mockResolvedValue({revision:8,items:[]});
  const query={category:"all",view:"recent",search:"核心概念",sort:{field:"title",direction:"ascending"},limit:20} as const;
  await expect(tauriNotePageCatalogGateway.queryNotePageCatalog(query)).resolves.toEqual({revision:8,items:[]});
  expect(f.invoke.mock.calls).toEqual([["query_note_page_catalog",{query}]]);
});
it("does not serialize cancellation into IPC or publish a canceled native result",async()=>{
  const query={category:"all",view:"recent"} as const, controller=new AbortController();
  controller.abort();
  await expect(tauriNotePageCatalogGateway.queryNotePageCatalog(query,{signal:controller.signal})).rejects.toMatchObject({name:"AbortError"});
  expect(f.invoke).not.toHaveBeenCalled();
  const active=new AbortController();let complete:(value:unknown)=>void=()=>{throw Error("not dispatched");};
  f.invoke.mockImplementation(()=>new Promise(resolve=>{complete=resolve;}));
  const result=tauriNotePageCatalogGateway.queryNotePageCatalog(query,{signal:active.signal});
  active.abort();complete({revision:5,items:[]});
  await expect(result).rejects.toMatchObject({name:"AbortError"});
  expect(f.invoke.mock.calls).toEqual([["query_note_page_catalog",{query}]]);
});

it("resumes durable native preparation until a complete result is available", async () => {
  const query={category:"all",view:"recent",search:"metadata"} as const;
  f.invoke.mockRejectedValueOnce({code:"catalog_preparing",message:"note_catalog_preparing"})
    .mockRejectedValueOnce({code:"catalog_preparing",message:"note_catalog_preparing"})
    .mockResolvedValue({revision:8,items:[]});
  await expect(tauriNotePageCatalogGateway.queryNotePageCatalog(query)).resolves.toEqual({revision:8,items:[]});
  expect(f.invoke.mock.calls).toEqual(Array.from({length:3},()=>["query_note_page_catalog",{query}]));
});
it("stops before the next native preparation batch when the query is hidden", async () => {
  const active=new AbortController(), query={category:"all",view:"recent",search:"metadata"} as const;
  f.invoke.mockImplementationOnce(async()=>{
    active.abort();
    throw {code:"catalog_preparing",message:"note_catalog_preparing"};
  });
  await expect(tauriNotePageCatalogGateway.queryNotePageCatalog(query,{signal:active.signal})).rejects.toMatchObject({name:"AbortError"});
  expect(f.invoke).toHaveBeenCalledTimes(1);
  f.invoke.mockResolvedValue({revision:9,items:[]});
  await expect(tauriNotePageCatalogGateway.queryNotePageCatalog(query)).resolves.toEqual({revision:9,items:[]});
  expect(f.invoke).toHaveBeenCalledTimes(2);
});

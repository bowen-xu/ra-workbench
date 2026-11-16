/** @vitest-environment happy-dom */
import { beforeEach, afterEach, it, expect, vi } from "vitest";
import { emptyNalState, parseNalConditions, NalInferenceCache, type NalState, type NalQueryCollection } from "@research-assistant/core/nal";
import { selectAppStorage } from "../../../../../apps/web/src/core/runtime/appStorage";
import { loadNalQueryCollections } from "../../../../../../plugins/ra-literature-library/src/services/nalQueryCollections";
import { getNalQueryGateway, NAL_QUERY_STORE_KEY } from "../../../../../../plugins/ra-literature-library/src/services/nalQueryGateway";
import type { NalQueryStore } from "@research-assistant/data/nal";
const vocabulary=["AI","perception","belongs"].map(id=>({atom:{kind:"concept" as const,id},label:id}));
const conditions=(id:string)=>parseNalConditions(`($x * ${id}) --> belongs`,vocabulary);
let storage:Storage;
beforeEach(()=>{const values=new Map<string,string>();storage={get length(){return values.size;},key:i=>[...values.keys()][i]??null,getItem:k=>values.get(k)??null,setItem:(k,v)=>{values.set(k,v);},removeItem:k=>{values.delete(k);},clear:()=>values.clear()};selectAppStorage(storage);});
afterEach(()=>vi.restoreAllMocks());
const state:NalState={...emptyNalState(),statements:[{id:"a",subject:{kind:"product",terms:[{kind:"paper",id:"p"},{kind:"concept",id:"AI"}]},predicate:{kind:"concept",id:"belongs"},origin:"human"}]};
it("upgrades saved query identities and reopens cached results without inference joins",async()=>{
  storage.setItem("research-assistant:nal-queries:v1",JSON.stringify([{id:"old",name:"AI",expression:"(?x × concept:AI) → concept:belongs"}]));
  const first=await loadNalQueryCollections(state,new Set(["p"]),vocabulary);
  expect(first.results()[0]?.paperIds).toEqual(["p"]);
  const before=storage.getItem(NAL_QUERY_STORE_KEY);
  const sync=vi.spyOn(NalInferenceCache.prototype,"sync");
  const second=await loadNalQueryCollections(state,new Set(["p"]),vocabulary);
  expect(second.results()[0]?.definition.id).toBe("old");
  expect(sync.mock.results.every(r=>r.type==="return" && r.value.joins===0)).toBe(true);
  expect(storage.getItem(NAL_QUERY_STORE_KEY)).toBe(before);
});
it("rebuilds a damaged derived cache and retains definitions; store CAS preserves another writer",async()=>{
  const def:NalQueryCollection={id:"a",name:"AI",conditions:conditions("AI")};
  await loadNalQueryCollections(state,new Set(["p"]),vocabulary,()=>[def]);
  const gateway=getNalQueryGateway(),stored=await gateway.read();
  await gateway.save({...stored,cache:{version:0}},stored.revision);
  const restored=await loadNalQueryCollections(state,new Set(["p"]),vocabulary);
  expect(restored.results()[0]?.paperIds).toEqual(["p"]);
  const next=await gateway.read();
  await expect(gateway.save({...next,definitions:[]},stored.revision)).rejects.toThrow("变化");
  expect((await gateway.read()).definitions).toEqual([def]);
});
it("persistently updates members on source deltas and leaves the cache intact on a write failure",async()=>{
  await loadNalQueryCollections(state,new Set(["p"]),vocabulary,()=>[{id:"a",name:"AI",conditions:conditions("AI")}]);
  const removed=await loadNalQueryCollections({...state,statements:[]},new Set(["p"]),vocabulary);
  expect(removed.results()[0]?.paperIds).toEqual([]);
  const before=storage.getItem(NAL_QUERY_STORE_KEY),write=storage.setItem;
  storage.setItem=()=>{throw new Error("disk full");};
  await expect(loadNalQueryCollections(state,new Set(["p"]),vocabulary)).rejects.toThrow("disk full");
  expect(storage.getItem(NAL_QUERY_STORE_KEY)).toBe(before);storage.setItem=write;
});
it("retains old saved queries when a referenced concept has already been deleted",async()=>{
  storage.setItem("research-assistant:nal-queries:v1",JSON.stringify([{id:"old",name:"已删除概念的查询",expression:"(?x × concept:deleted-domain) → concept:belongs"}]));
  const projection=await loadNalQueryCollections(state,new Set(["p"]),vocabulary);
  expect(projection.results()[0]?.definition.name).toBe("已删除概念的查询");
  expect(projection.results()[0]?.paperIds).toEqual([]);
});

it("reloads concurrent definitions and revisions on each of three CAS attempts", async () => {
  const definition = (id: string): NalQueryCollection => ({id, name: id, conditions: conditions("AI")});
  const snapshots: readonly [NalQueryStore, NalQueryStore, NalQueryStore] = [
    {revision: 1, definitions: [definition("initial")], cache: null, legacyImported: true},
    {revision: 4, definitions: [definition("other-writer")], cache: null, legacyImported: true},
    {revision: 9, definitions: [definition("other-writer"), definition("third-writer")], cache: null, legacyImported: true},
  ];
  const gateway = getNalQueryGateway();
  const read = vi.spyOn(gateway, "read")
    .mockResolvedValueOnce(snapshots[0])
    .mockResolvedValueOnce(snapshots[1])
    .mockResolvedValueOnce(snapshots[2]);
  const save = vi.spyOn(gateway, "save")
    .mockRejectedValueOnce(new Error("查询集合已变化，请重试"))
    .mockRejectedValueOnce(new Error("查询集合已变化，请重试"))
    .mockImplementationOnce(async (value, revision) => ({...value, revision: revision + 1}));
  const local = definition("local-edit");
  const result = await loadNalQueryCollections(state, new Set(["p"]), vocabulary,
    definitions => [...definitions, local]);
  expect(read).toHaveBeenCalledTimes(3);
  expect(save.mock.calls.map(([value, revision]) => ({revision, ids: value.definitions.map(d => d.id)}))).toEqual([
    {revision: 1, ids: ["initial", "local-edit"]},
    {revision: 4, ids: ["other-writer", "local-edit"]},
    {revision: 9, ids: ["other-writer", "third-writer", "local-edit"]},
  ]);
  expect(result.results().map(entry => entry.definition.id)).toEqual(["other-writer", "third-writer", "local-edit"]);
});

it("stops after the third confirmed CAS conflict and preserves the last error", async () => {
  const gateway = getNalQueryGateway();
  const snapshot: NalQueryStore = {revision: 3, definitions: [], cache: null, legacyImported: true};
  const conflict = new Error("查询集合已变化，请重试");
  const read = vi.spyOn(gateway, "read").mockResolvedValue(snapshot);
  const save = vi.spyOn(gateway, "save").mockRejectedValue(conflict);
  await expect(loadNalQueryCollections(state, new Set(["p"]), vocabulary)).rejects.toBe(conflict);
  expect(read).toHaveBeenCalledTimes(3);
  expect(save).toHaveBeenCalledTimes(3);
  expect(save.mock.calls.map(([, revision]) => revision)).toEqual([3, 3, 3]);
  expect(storage.getItem(NAL_QUERY_STORE_KEY)).toBeNull();
});

it("never retries an unknown commit result whose message lacks the confirmed CAS phrase", async () => {
  const gateway = getNalQueryGateway();
  const snapshot: NalQueryStore = {revision: 6, definitions: [], cache: null, legacyImported: true};
  const unknown = Object.assign(new Error("查询集合提交或回滚结果未知，禁止自动重试。"), {
    queryOutcome: {status: "unknown", observedRevision: 7},
  });
  const read = vi.spyOn(gateway, "read").mockResolvedValue(snapshot);
  const save = vi.spyOn(gateway, "save").mockRejectedValue(unknown);
  await expect(loadNalQueryCollections(state, new Set(["p"]), vocabulary)).rejects.toBe(unknown);
  expect(read).toHaveBeenCalledTimes(1);
  expect(save).toHaveBeenCalledTimes(1);
  expect(storage.getItem(NAL_QUERY_STORE_KEY)).toBeNull();
});

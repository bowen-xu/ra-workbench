/** @vitest-environment happy-dom */
import { beforeEach, expect, it } from "vitest";
import { emptyLiteratureDraft } from "@research-assistant/core/literature";
import { selectAppRuntime } from "../../../core/runtime/appRuntime";
import { getNalGateway } from "../../../../../../plugins/literature_library/src/services/nalGateway";
import { getResearchKnowledgeGateway } from "@research-assistant/literature-library-plugin/knowledgeGateway";
import { getLiteratureGateway } from "@research-assistant/literature-library-plugin/literatureGateway";
import { nalPreviewDefinition } from "../../../../../../plugins/literature_library/src/services/nalLegacyCleanup";
import { seedNalMockData } from "./seedNalMockData";
import { getNalQueryGateway } from "../../../../../../plugins/literature_library/src/services/nalQueryGateway";

class MemoryStorage implements Storage {
  private values = new Map<string,string>();
  get length() {return this.values.size;}
  getItem(key:string) {return this.values.get(key) ?? null;}
  setItem(key:string,value:string) {this.values.set(key,value);}
  removeItem(key:string) {this.values.delete(key);}
  key(index:number) {return [...this.values.keys()][index] ?? null;}
  clear() {this.values.clear();}
}
let storage: Storage;
beforeEach(()=>{
  storage = new MemoryStorage();
  selectAppRuntime({mode:"browser",storage,gateways:{},dispose(){}});
});
const paper = () => getLiteratureGateway().save({...emptyLiteratureDraft(),title:"合成文献"});

it("concurrent and repeated starts create exactly one set of concepts and statements", async()=>{
  const item = await paper();
  await Promise.all(Array.from({length:4},()=>seedNalMockData(storage,item.id)));
  const first = await getNalGateway().read();
  expect(first.concepts).toHaveLength(8);
  expect(new Set(first.concepts.map(c=>c.label)).size).toBe(8);
  expect(first.statements).toHaveLength(3);
  const queries=await getNalQueryGateway().read();
  expect(queries.definitions.map(d=>d.name)).toEqual(["AI领域文献","深度学习领域文献"]);
  expect(queries.definitions[1]?.parentId).toBe(queries.definitions[0]?.id);
  await seedNalMockData(storage,item.id);
  expect(await getNalGateway().read()).toEqual(first);
});

it("does not recreate deleted example queries and safely retries a failed example save",async()=>{
  const item=await paper(),queries=getNalQueryGateway();
  const ports={knowledge:getResearchKnowledgeGateway(),nal:getNalGateway(),queries:{...queries,save:async()=>{throw new Error("模拟查询保存失败");}}};
  await expect(seedNalMockData(storage,item.id,ports)).rejects.toThrow("模拟查询保存失败");
  expect(storage.getItem("nal-query-examples-seeded")).toBeNull();
  await seedNalMockData(storage,item.id);
  const created=await queries.read();
  expect(created.definitions).toHaveLength(2);
  await queries.save({definitions:[],cache:null,legacyImported:true},created.revision);
  await seedNalMockData(storage,item.id);
  expect((await queries.read()).definitions).toEqual([]);
});

it("reuses an interrupted older fixture without merging a context-qualified concept", async()=>{
  const item = await paper(), knowledge = getResearchKnowledgeGateway();
  const older = await knowledge.saveConcept({preferredTerm:"AI领域",label:"AI领域",explanation:nalPreviewDefinition("AI领域"),disambiguation:"预览夹具",domain:"",tradition:"",period:""});
  await knowledge.saveConcept({preferredTerm:"AI领域",label:"AI领域`我的语境",explanation:"我的领域定义",disambiguation:"人工定义",domain:"",tradition:"",period:""});
  let calls = 0;
  const ports = {nal:getNalGateway(),knowledge:{saveConcept:async(...args:Parameters<typeof knowledge.saveConcept>)=>{
    if (++calls === 3) throw new Error("模拟中断");
    return knowledge.saveConcept(...args);
  }}};
  await expect(seedNalMockData(storage,item.id,ports)).rejects.toThrow("模拟中断");
  expect(storage.getItem("nal-fixture-seeded")).toBeNull();
  await seedNalMockData(storage,item.id,ports);
  const result = await getNalGateway().read();
  expect(result.concepts).toHaveLength(9);
  expect(result.concepts.filter(c=>c.label.startsWith("AI领域"))).toHaveLength(2);
  expect(result.statements.some(s=>JSON.stringify(s).includes(older.id))).toBe(true);
});

it("a failure after saving statements does not duplicate them on retry", async()=>{
  const item = await paper(), nal = getNalGateway();
  let interrupted = false;
  const ports = {knowledge:getResearchKnowledgeGateway(),nal:{...nal,mutate:async(...args:Parameters<typeof nal.mutate>)=>{
    if (!interrupted && args[0].kind === "set_transitivity") {interrupted=true;throw new Error("规则保存中断");}
    return nal.mutate(...args);
  }}};
  await expect(seedNalMockData(storage,item.id,ports)).rejects.toThrow("规则保存中断");
  await seedNalMockData(storage,item.id,ports);
  const result = await nal.read();
  expect(result.concepts).toHaveLength(8);
  expect(result.statements).toHaveLength(3);
  expect(result.rules).toHaveLength(1);
});

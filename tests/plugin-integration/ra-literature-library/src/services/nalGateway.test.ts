/** @vitest-environment happy-dom */
import { beforeEach,afterEach,expect,it,vi } from "vitest";
import { emptyLiteratureDraft } from "@research-assistant/core/literature";
import { parseNal,queryNal,type NalVocabulary } from "@research-assistant/core/nal";
import { getNalGateway } from "../../../../../../plugins/ra-literature-library/src/services/nalGateway";
import { getLiteratureGateway } from "../../../../../../plugins/ra-literature-library/src/services/literatureGateway";
import { getResearchKnowledgeGateway } from "../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
import { getCollectionGateway } from "../../../../../../plugins/ra-literature-library/src/services/collectionGateway";
import { getNotesGateway } from "../../../../../apps/web/src/core/features/notes/gateways/index";
import { purgeBrowserNal } from "../../../../../../plugins/ra-literature-library/src/services/browserNalStorage";
import { browserKnowledgeStorage } from "../../../../../../plugins/ra-literature-library/src/services/browserKnowledgeStorage";
import * as runtime from "../../../../../apps/web/src/core/runtime/appRuntime";
import { nalPreviewDefinition } from "../../../../../../plugins/ra-literature-library/src/services/nalLegacyCleanup";
beforeEach(()=>{const values=new Map<string,string>();const storage={getItem:(k:string)=>values.get(k) ?? null,setItem:(k:string,v:string)=>{values.set(k,v);},removeItem:(k:string)=>values.delete(k),clear:()=>values.clear()};vi.stubGlobal("localStorage",storage);Object.defineProperty(window,"localStorage",{configurable:true,value:storage});});
afterEach(()=>{vi.unstubAllGlobals();vi.restoreAllMocks();});
const concept=(label:string)=>getResearchKnowledgeGateway().saveConcept({preferredTerm:label,label,explanation:`定义：${label}`,disambiguation:"",domain:"",tradition:"",period:""});
it("persistently deletes an obsolete placeholder and does not restore it on reload",async()=>{
  const gateway=getNalGateway();
  await gateway.read();
  const old=await getResearchKnowledgeGateway().saveConcept({preferredTerm:"related-field",label:"related-field",explanation:"旧集合关系。反向显示：related-field；有向：false。",disambiguation:"旧迁移",domain:"",tradition:"",period:""});
  const state=await gateway.read();
  expect(state.concepts.some(c=>c.id === old.id)).toBe(false);
  expect((await getResearchKnowledgeGateway().searchConcepts("related-field"))).toEqual([]);
  expect(JSON.parse(localStorage.getItem("research-assistant.nal.v1")!).concepts.some((c:{id:string})=>c.id === old.id)).toBe(false);
  expect(await gateway.read()).toEqual(state);
  const replacement=await concept("related-field");
  expect(replacement.id).not.toBe(old.id);
  expect((await gateway.read()).concepts.some(c=>c.id === replacement.id)).toBe(true);
});

it("deletes old preview migration identities and duplicate fixtures from active storage and their relationships",async()=>{
  vi.spyOn(runtime,"isAppPreview").mockReturnValue(true);
  const gateway=getNalGateway(),knowledge=getResearchKnowledgeGateway();
  await gateway.read();
  const active=await knowledge.saveConcept({preferredTerm:"AI领域",label:"AI领域",explanation:nalPreviewDefinition("AI领域"),disambiguation:"预览夹具",domain:"",tradition:"",period:""});
  const retired=await knowledge.saveConcept({preferredTerm:"原归类",label:"原归类",explanation:"旧迁移",disambiguation:"旧迁移",domain:"",tradition:"",period:""});
  const manual=await concept("原归类`我的语境");
  const note=await getNotesGateway().createPage({title:"保留笔记正文"});
  const stored=browserKnowledgeStorage.readConcepts();
  localStorage.setItem("research-assistant.concepts.v1",JSON.stringify([...stored,{...active,id:"duplicate"}]));
  localStorage.setItem("research-assistant.nal.legacy-map.v1",JSON.stringify({membership:retired.id}));
  const previous=JSON.parse(localStorage.getItem("research-assistant.nal.v1")!);
  localStorage.setItem("research-assistant.nal.v1",JSON.stringify({...previous,statements:[
    {id:"kept",origin:"human",subject:{kind:"concept",id:active.id},predicate:{kind:"concept",id:manual.id}},
    {id:"removed",origin:"legacy",subject:{kind:"concept",id:retired.id},predicate:{kind:"concept",id:manual.id}},
  ],annotations:[{id:"old-note",targetKind:"statement",targetId:"removed",notePageId:note.page.id}]}));
  const next=await gateway.read();
  expect(new Set(next.concepts.map(c=>c.id))).toEqual(new Set([active.id,manual.id]));
  expect(next.statements.map(s=>s.id)).toEqual(["kept"]);
  expect(next.annotations).toEqual([]);
  expect((await knowledge.searchConcepts("AI领域")).map(c=>c.id)).toEqual([active.id]);
  expect((await getNotesGateway().loadDocument(note.page.id)).page.title).toBe("保留笔记正文");
  expect(await gateway.read()).toEqual(next);
});

it("rolls back cleanup deletion records when saving the cleaned graph fails",async()=>{
  const gateway=getNalGateway();
  await gateway.read();
  const old=await getResearchKnowledgeGateway().saveConcept({preferredTerm:"related-field",label:"related-field",explanation:"旧集合关系。反向显示：related-field；有向：false。",disambiguation:"旧迁移",domain:"",tradition:"",period:""});
  const before=localStorage.getItem("research-assistant.nal.v1"),write=localStorage.setItem.bind(localStorage);
  const fault=vi.spyOn(localStorage,"setItem").mockImplementation((key,value)=>{if(key === "research-assistant.nal.v1")throw new Error("存储失败");write(key,value);});
  await expect(gateway.read()).rejects.toThrow("存储失败");
  expect(localStorage.getItem("research-assistant.nal.v1")).toBe(before);
  expect(localStorage.getItem("research-assistant.concept-deletions.v1")).toBeNull();
  expect(browserKnowledgeStorage.readConcepts().some(c=>c.id === old.id)).toBe(true);
  fault.mockRestore();
  expect((await gateway.read()).concepts.some(c=>c.id === old.id)).toBe(false);
});
it("persists inheritance queries, rejects concurrent stale writes, and keeps annotations in the notes store",async()=>{
  const paper=await getLiteratureGateway().save({...emptyLiteratureDraft(),title:"文章3"});
  const [ai,deep,belongs]=await Promise.all([concept("AI领域"),concept("深度学习领域"),concept("所属领域")]);
  const gateway=getNalGateway();let state=await gateway.read();
  state=await gateway.mutate({kind:"add_statements",statements:[
    {id:"inherit",subject:{kind:"product",terms:[{kind:"concept",id:deep.id},{kind:"concept",id:ai.id}]},predicate:{kind:"concept",id:belongs.id},origin:"human"},
    {id:"paper",subject:{kind:"product",terms:[{kind:"paper",id:paper.id},{kind:"concept",id:deep.id}]},predicate:{kind:"concept",id:belongs.id},origin:"human"},
  ]},state.revision);
  state=await gateway.mutate({kind:"set_concept_kind",conceptId:belongs.id,conceptKind:"relation"},state.revision);
  state=await gateway.mutate({kind:"set_transitivity",relationConceptId:belongs.id,enabled:true},state.revision);
  const vocabulary:readonly NalVocabulary[]=state.concepts.map(c=>({atom:{kind:"concept",id:c.id},label:c.label}));
  expect(queryNal(state,parseNal("(?文章 × AI领域) → 所属领域",vocabulary,true)).find(m=>m.paperIds.includes(paper.id))?.paperIds).toEqual([paper.id]);
  const page=await getNotesGateway().createPage({title:"关系注释"});
  const results=await Promise.allSettled([
    gateway.mutate({kind:"link_annotation",annotation:{id:"a",targetKind:"statement",targetId:"paper",notePageId:page.page.id}},state.revision),
    gateway.mutate({kind:"delete_statement",id:"inherit"},state.revision),
  ]);
  expect(results.filter(r=>r.status === "fulfilled")).toHaveLength(1);
  state=await gateway.read();expect(state.annotations).toHaveLength(1);
  await getLiteratureGateway().trashMany([paper.id]);expect((await gateway.read()).statements).toHaveLength(2);
  purgeBrowserNal([paper.id]);state=await gateway.read();expect(state.annotations).toHaveLength(0);expect(state.statements).toHaveLength(1);
  expect((await getNotesGateway().loadDocument(page.page.id)).page.title).toBe("关系注释");
});
it("imports old collection identities independently even when both reference one knowledge concept",async()=>{
  const c=await concept("既有概念"),old=getCollectionGateway(),gateway=getNalGateway();let state=await old.read();
  for(const id of ["a","b"])state=await old.mutate({kind:"save_collection",collection:{id,name:"同名归类",description:id,conceptId:c.id}},state.revision);
  const imported=await gateway.read();
  expect(imported.concepts.filter(c=>c.label.startsWith("同名归类"))).toHaveLength(2);
  expect(new Set(imported.concepts.map(c=>c.label)).size).toBe(imported.concepts.length);
  expect(imported.statements.filter(s=>s.subject.kind === "product")).toHaveLength(2);
  expect((await gateway.read()).statements).toEqual(imported.statements);
  expect(await old.read()).toEqual(state);
});
it("refreshes a stale graph revision after an external concept edit and rejects malformed persisted data",async()=>{
  const gateway=getNalGateway(),initial=await gateway.read();await concept("新增概念");
  expect((await gateway.read()).revision).toBeGreaterThan(initial.revision);
  await expect(gateway.mutate({kind:"unlink_annotation",id:"none"},initial.revision)).rejects.toThrow("变化");
  window.localStorage.setItem("research-assistant.nal.v1","bad-json");await expect(gateway.read()).rejects.toThrow();
});

it("upgrades v1 legacy arrows and rules while retaining human facts and annotation targets",async()=>{
  const a=await concept("深度学习领域"),b=await concept("AI领域"),gateway=getNalGateway();
  const note=await getNotesGateway().createPage({title:"历史 annotation"});
  const state=await gateway.read();
  const persisted:Record<string,unknown>={...state,statements:[
    {id:"imported",origin:"legacy",subject:{kind:"concept",id:a.id},predicate:{kind:"concept",id:b.id}},
    {id:"human",origin:"human",subject:{kind:"concept",id:b.id},predicate:{kind:"concept",id:a.id}},
  ],annotations:[{id:"annotation",targetKind:"statement",targetId:"imported",notePageId:note.page.id}]};
  delete persisted.rules;delete persisted.relationSemanticsVersion;
  window.localStorage.setItem("research-assistant.nal.v1",JSON.stringify(persisted));
  const repaired=await gateway.read();
  expect(repaired.statements.find(s=>s.id === "imported")?.subject.kind).toBe("product");
  expect(repaired.statements.find(s=>s.id === "human")?.subject).toEqual({kind:"concept",id:b.id});
  expect(repaired.annotations[0]?.targetId).toBe("imported");
  expect(repaired.rules).toEqual([]);
  expect((await gateway.read()).revision).toBe(repaired.revision);
  const typed=await gateway.mutate({kind:"set_concept_kind",conceptId:b.id,conceptKind:"relation"},repaired.revision);
  const enabled=await gateway.mutate({kind:"set_transitivity",relationConceptId:b.id,enabled:true},typed.revision);
  expect((await gateway.read()).rules).toEqual(enabled.rules);
  browserKnowledgeStorage.writeConcepts(browserKnowledgeStorage.readConcepts().filter(c=>c.id !== b.id));
  const deleted=await gateway.read();
  expect(deleted.rules).toEqual([]);
  expect(deleted.statements).toEqual([]);
  expect(deleted.annotations).toEqual([]);
  expect((await getNotesGateway().loadDocument(note.page.id)).page.title).toBe("历史 annotation");
});
it("creates concepts without explanations and deletes their active relationships while preserving notes and history",async()=>{
  const knowledge=getResearchKnowledgeGateway(),gateway=getNalGateway();
  const a=await knowledge.saveConcept({preferredTerm:"临时概念",label:"临时概念",explanation:"",disambiguation:"",domain:"",tradition:"",period:""});
  expect(a.explanation).toBe("");
  const b=await concept("保留概念"),note=await getNotesGateway().createPage({title:"保留笔记"});
  let state=await gateway.read();
  state=await gateway.mutate({kind:"add_statements",statements:[{id:"delete-related",origin:"human",subject:{kind:"product",terms:[{kind:"concept",id:b.id},{kind:"product",terms:[{kind:"concept",id:a.id},{kind:"concept",id:b.id}]}]},predicate:{kind:"concept",id:b.id}}]},state.revision);
  state=await gateway.mutate({kind:"link_annotation",annotation:{id:"delete-ann",targetKind:"statement",targetId:"delete-related",notePageId:note.page.id}},state.revision);
  const previous=state.revision;
  state=await gateway.mutate({kind:"delete_concept",id:a.id},previous);
  expect(state.concepts.some(c=>c.id===a.id)).toBe(false);
  expect(state.statements).toHaveLength(0);
  expect(state.annotations).toHaveLength(0);
  expect((await knowledge.searchConcepts("临时概念"))).toHaveLength(0);
  expect((await getNotesGateway().loadDocument(note.page.id)).page.title).toBe("保留笔记");
  await expect(gateway.mutate({kind:"delete_concept",id:b.id},previous)).rejects.toThrow("变化");
  const replacement=await knowledge.saveConcept({preferredTerm:"临时概念",label:"临时概念",explanation:"",disambiguation:"",domain:"",tradition:"",period:""});
  expect(replacement.id).not.toBe(a.id);
  expect((await gateway.read()).concepts.some(c=>c.id===a.id)).toBe(false);
  await expect(knowledge.saveConcept({id:a.id,expectedRevision:a.revision,preferredTerm:"临时概念",label:"其他名称",explanation:"",disambiguation:"",domain:"",tradition:"",period:""})).rejects.toThrow("已删除");
});

it("creates one annotation across repeated/concurrent requests, including after trash and retry",async()=>{
  const c=await concept("单一笔记"),gateway=getNalGateway(),notes=getNotesGateway();
  const request={targetKind:"concept" as const,targetId:c.id,title:c.label};
  const [a,b]=await Promise.all([gateway.ensureAnnotation(request),gateway.ensureAnnotation(request)]);
  expect(a).toEqual(b);
  expect((await notes.listPages()).filter(p=>p.title===c.label)).toHaveLength(1);
  const current=await notes.loadDocument(a.notePageId as import("@research-assistant/core/notes").NotePageId);
  await notes.setPageStatus({id:current.page.id,status:"trashed",expectedRevision:current.page.revision});
  expect(await gateway.ensureAnnotation(request)).toEqual(a);
  const other=await notes.createPage({title:"额外笔记"}),state=await gateway.read();
  await expect(gateway.mutate({kind:"link_annotation",annotation:{...a,id:"another",notePageId:other.page.id}},state.revision)).rejects.toThrow("只能关联一份");
  expect((await notes.loadDocument(other.page.id)).page.title).toBe("额外笔记");
});

it("keeps all old note documents while persistently reducing multiple links to one",async()=>{
  const c=await concept("旧笔记"),gateway=getNalGateway(),notes=getNotesGateway();
  const [a,b]=await Promise.all([notes.createPage({title:"旧笔记一"}),notes.createPage({title:"旧笔记二"})]);
  const state=await gateway.read();
  localStorage.setItem("research-assistant.nal.v1",JSON.stringify({...state,annotations:[a,b].map((n,i)=>({id:String(i),targetKind:"concept",targetId:c.id,notePageId:n.page.id}))}));
  const upgraded=await gateway.read();
  expect(upgraded.annotations.map(a=>a.notePageId)).toEqual([a.page.id]);
  expect(await gateway.read()).toEqual(upgraded);
  expect((await notes.loadDocument(b.page.id)).page.title).toBe("旧笔记二");
});

it("reuses a saved page after a failed link and rejects missing targets without creating a page",async()=>{
  const c=await concept("失败后重试"),gateway=getNalGateway(),notes=getNotesGateway();
  const request={targetKind:"concept" as const,targetId:c.id,title:c.label};
  await gateway.read();
  const write=localStorage.setItem.bind(localStorage);
  const fault=vi.spyOn(localStorage,"setItem").mockImplementation((key,value)=>{if(key==="research-assistant.nal.v1")throw new Error("模拟关联写入失败");write(key,value);});
  await expect(gateway.ensureAnnotation(request)).rejects.toThrow("模拟关联写入失败");
  fault.mockRestore();
  const saved=(await notes.listPages()).filter(p=>p.title===c.label);expect(saved).toHaveLength(1);
  expect((await gateway.ensureAnnotation(request)).notePageId).toBe(saved[0]!.id);
  expect((await notes.listPages()).filter(p=>p.title===c.label)).toHaveLength(1);
  await expect(gateway.ensureAnnotation({...request,targetId:"missing"})).rejects.toThrow("已不存在");
  expect(await notes.listPages()).toHaveLength(1);
});

import type { NalGateway, NalQueryGateway } from "@research-assistant/data/nal";
import { nalAtoms } from "@research-assistant/core/nal";
import type { ResearchKnowledgeGateway } from "@research-assistant/literature-library-plugin/knowledgeGateway";
import { getResearchKnowledgeGateway } from "@research-assistant/literature-library-plugin/knowledgeGateway";
import { getNalGateway } from "../../../../../../plugins/literature_library/src/services/nalGateway";
import { nalPreviewDefinition, nalPreviewDefinitionKey } from "../../../../../../plugins/literature_library/src/services/nalLegacyCleanup";
import { getNalQueryGateway } from "../../../../../../plugins/literature_library/src/services/nalQueryGateway";
import { seedNalQueryExamples } from "./seedNalQueryExamples";

const pending = new WeakMap<Storage, Promise<void>>();
const labels = ["领域","AI领域","深度学习领域","所属领域","方法","active inference方法","基于方法","研究发展"] as const;
const relationLabels = new Set(["所属领域","基于方法","研究发展"]);
const fixtureId = (n: number) => `21000000-0000-4000-8000-${String(n).padStart(12,"0")}`;
interface SeedPorts { readonly knowledge: Pick<ResearchKnowledgeGateway,"saveConcept">; readonly nal: NalGateway;readonly queries?:NalQueryGateway }

async function seed(storage: Storage, paperId: string, ports: SeedPorts): Promise<void> {
  if (storage.getItem("nal-fixture-seeded") === "1") {await classifyFixtures(ports);return;}
  const state = await ports.nal.read();
  const referenced = new Set(state.statements.flatMap(s=>[...nalAtoms(s.subject),...nalAtoms(s.predicate)]).map(a=>a.id));
  const ids: Record<string,string> = {};
  for (const [i,label] of labels.entries()) {
    // Resume partially saved older fixtures; never match an ordinary same-name concept.
    const candidates = state.concepts.filter(c=>nalPreviewDefinitionKey(c) === label);
    const existing = state.concepts.find(c=>c.id === fixtureId(i+1)) ?? candidates.find(c=>referenced.has(c.id)) ?? candidates[0];
    ids[label] = existing?.id ?? (await ports.knowledge.saveConcept({id:fixtureId(i+1),preferredTerm:label,label,
      explanation:nalPreviewDefinition(label),disambiguation:"预览夹具",domain:"",tradition:"",period:""})).id;
  }
  const atom = (label: typeof labels[number]) => ({kind:"concept" as const,id:ids[label]!});
  const current = await ports.nal.read();
  await ports.nal.mutate({kind:"add_statements",statements:[
    {id:fixtureId(101),subject:{kind:"product",terms:[atom("深度学习领域"),atom("AI领域")]},predicate:atom("所属领域"),origin:"human"},
    {id:fixtureId(102),subject:{kind:"product",terms:[{kind:"paper",id:paperId},atom("深度学习领域")]},predicate:atom("所属领域"),origin:"human"},
    {id:fixtureId(103),subject:atom("active inference方法"),predicate:atom("方法"),origin:"human"},
  ]},current.revision);
  const next=await classifyFixtures(ports);
  await ports.nal.mutate({kind:"set_transitivity",relationConceptId:ids["所属领域"]!,enabled:true},next.revision);
  storage.setItem("nal-fixture-seeded","1");
}

async function classifyFixtures(ports: SeedPorts) {
  let state=await ports.nal.read();
  for (const concept of state.concepts) {
    const fixture=nalPreviewDefinitionKey(concept);
    if(fixture !== undefined && relationLabels.has(fixture) && concept.conceptKind !== "relation") {
      state=await ports.nal.mutate({kind:"set_concept_kind",conceptId:concept.id,conceptKind:"relation"},state.revision);
    }
  }
  return state;
}

/** Concurrent starts share one operation; failed starts can safely resume. */
export function seedNalMockData(storage: Storage, paperId: string, ports: SeedPorts = {
  knowledge:getResearchKnowledgeGateway(),nal:getNalGateway(),
}): Promise<void> {
  const running = pending.get(storage);
  if (running) return running;
  const operation = seed(storage,paperId,ports).then(async()=>{
    await seedNalQueryExamples(storage,await ports.nal.read(),ports.queries ?? getNalQueryGateway());
  }).finally(()=>{
    if (pending.get(storage) === operation) pending.delete(storage);
  });
  pending.set(storage,operation);
  return operation;
}

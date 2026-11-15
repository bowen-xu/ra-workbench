import type { NalState, NalQueryCollection } from "@research-assistant/core/nal";
import type { NalQueryGateway } from "@research-assistant/data/nal";
import { nalPreviewDefinitionKey } from "../../../../../../plugins/literature_library/src/services/nalLegacyCleanup";

const KEY="nal-query-examples-seeded";
const parentId="21000000-0000-4000-8000-000000000201",childId="21000000-0000-4000-8000-000000000202";

/** Preview examples are ordinary saved queries; explicit deletion survives subsequent starts. */
export async function seedNalQueryExamples(storage:Storage,state:NalState,gateway:NalQueryGateway):Promise<void> {
  if(storage.getItem(KEY) === "1")return;
  const find=(label:string)=>state.concepts.find(c=>nalPreviewDefinitionKey(c) === label);
  const ai=find("AI领域"),deep=find("深度学习领域"),relation=find("所属领域");
  if(!ai || !deep || !relation){storage.setItem(KEY,"1");return;}
  const condition=(id:string)=>({subject:{kind:"product" as const,terms:[{kind:"variable" as const,name:"文章"},{kind:"concept" as const,id}]},predicate:{kind:"concept" as const,id:relation.id}});
  const examples:readonly NalQueryCollection[]=[
    {id:parentId,name:"AI领域文献",conditions:[condition(ai.id)]},
    {id:childId,parentId,name:"深度学习领域文献",conditions:[condition(deep.id)]},
  ];
  for(let attempt=0;attempt<3;attempt++){
    const stored=await gateway.read();
    const missing=examples.filter(e=>!stored.definitions.some(d=>d.id === e.id));
    if(!missing.length){storage.setItem(KEY,"1");return;}
    try{
      await gateway.save({definitions:[...stored.definitions,...missing],cache:null,legacyImported:stored.legacyImported},stored.revision);
    }catch(error){
      if(!(error instanceof Error) || !error.message.includes("查询集合已变化") || attempt===2)throw error;
      continue;
    }
    storage.setItem(KEY,"1");return;
  }
}

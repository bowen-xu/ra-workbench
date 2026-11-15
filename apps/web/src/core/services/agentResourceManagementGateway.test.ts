import {expect,it} from "vitest";
import {decodeSkillImportPreview} from "./agentResourceManagementGateway";
it("validates imported source identities and exact resolved commits",()=>{
 const candidate={sourceId:".agents/skills/review/SKILL.md",name:"review",revision:"r",existingRevision:null};const valid={importId:crypto.randomUUID(),candidates:[candidate],resolvedCommit:"a".repeat(40)};
 expect(decodeSkillImportPreview(valid)).toEqual(valid);expect(()=>decodeSkillImportPreview({...valid,candidates:[candidate,candidate]})).toThrow("重复");expect(()=>decodeSkillImportPreview({...valid,resolvedCommit:"HEAD"})).toThrow();expect(()=>decodeSkillImportPreview({...valid,candidates:[{...candidate,revision:null}]})).toThrow();
});

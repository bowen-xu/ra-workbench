import {expect,it} from "vitest";
import {toolActivityRows} from "./toolActivityModel";
import {toolEditDiagnostics} from "./toolEditDiagnostics";
it("selects causes and stable diagnostics without exposing full arguments or document bodies",()=>{
 const result={status:"failed",operationId:"operation-synthetic",httpStatus:403,patch:"DO NOT SHOW RETURNED PATCH",content:"DO NOT SHOW BODY",targets:[{path:"source://notes/private-id",name:"Resource",message:"Draft retained",structuredResource:{status:"invalid",resourceId:"private-id",draftId:"draft-synthetic",draftRevision:2,errors:[{path:"$.claims[0].quote",code:"invalid_evidence",message:"Synthetic evidence mismatch"}]}}]};
 const row=toolActivityRows([{type:"tool-call",callId:"a",name:"research_apply_patch",content:JSON.stringify({patch:"DO NOT SHOW INPUT PATCH"})},{type:"tool-result",callId:"a",name:"research_apply_patch",content:JSON.stringify(result)}])[0]!;
 const shown=toolEditDiagnostics(row).join("\n");
 for(const text of ["operation-synthetic","403","draft-synthetic","2","invalid_evidence","$.claims[0].quote","Synthetic evidence mismatch"])expect(shown).toContain(text);
 expect(shown).not.toContain("DO NOT SHOW");expect(shown).not.toContain("source://");expect(shown).not.toContain("private-id");
 expect(row.call?.content).toContain("DO NOT SHOW INPUT PATCH");expect(row.result?.content).toBe(JSON.stringify(result));
});

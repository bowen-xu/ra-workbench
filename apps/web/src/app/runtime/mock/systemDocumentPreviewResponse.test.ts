/** @vitest-environment happy-dom */
import {expect,it} from "vitest";
import {type AgentContextPreview,systemDocumentReference} from "@research-assistant/core/agent";
import {createMockSystemDocuments} from "./mockSystemDocuments";
import {systemDocumentPreviewResponse} from "./systemDocumentPreviewResponse";
import {WritingDiffDemoHistory} from "../../previews/agent/WritingDiffDemoHistory";
it("approved mock tool writes complete an ordinary durable Agent turn",async()=>{
 localStorage.clear();const documents=createMockSystemDocuments(localStorage),doc=await documents.gateway.read("structured-task");
 const preview:AgentContextPreview={schemaVersion:1,conversationId:crypto.randomUUID(),requestId:crypto.randomUUID(),turnId:crypto.randomUUID(),contextSnapshotId:crypto.randomUUID(),input:{text:"新增说明：测试规则",context:[],reasoningEffort:null,model:{providerId:"mock",modelId:"mock",revision:null},userInput:[{type:"text",text:"新增说明：测试规则",textElements:[]},systemDocumentReference(doc)]}};
 const history=new WritingDiffDemoHistory();const admission={scope:{manuscriptId:"independent",conversationId:preview.conversationId,requestId:preview.requestId,contextSnapshotId:preview.contextSnapshotId},turnId:preview.turnId};history.start(admission,preview.input.text,preview.input.model);
 const response=systemDocumentPreviewResponse(documents);expect(response.supports(preview)).toBe(true);
 const run=response.run(preview,history,admission,new AbortController().signal);
 for(let i=0;i<20&&!(await documents.gateway.pending(preview.conversationId)).length;i++)await new Promise(resolve=>setTimeout(resolve,0));
 await documents.gateway.decide(preview.conversationId,(await documents.gateway.pending(preview.conversationId))[0]!.id,true);
 await run;history.complete(admission,[],true);
 const result=await history.gateway.readHistory({conversationId:preview.conversationId,afterSeq:0,limit:100});
 expect(result.events.at(-1)?.payload).toEqual({type:"turnFinished",turnId:preview.turnId,outcome:"completed"});
 expect((await documents.gateway.read(doc.id)).content).toContain("测试规则");
});

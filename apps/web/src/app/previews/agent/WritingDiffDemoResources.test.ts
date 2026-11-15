import {afterEach,expect,it,vi} from "vitest";
import {agentDraftInput,insertAgentReference,type AgentInputDraft,type AgentSteeringHandoffIntent} from "@research-assistant/core/agent";
import {WritingDiffDemoStorage} from "./WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "./WritingDiffDemoAgentStorage";
import {WritingDiffDemoSubmission} from "./WritingDiffDemoSubmission";
import {WritingDiffDemoQueue} from "./WritingDiffDemoQueue";
const manuscriptId=crypto.randomUUID(),conversationId=crypto.randomUUID();
const instances:WritingDiffDemoSubmission[]=[];
afterEach(()=>{instances.splice(0).forEach(value=>value.dispose());vi.useRealTimers();});
async function fixture(){const storage=new WritingDiffDemoStorage(manuscriptId,[{path:"main.tex",before:"before",after:"before",kind:"modified",additions:0,deletions:0}]);const agent=new WritingDiffDemoAgentStorage(storage,undefined),execution=new WritingDiffDemoSubmission(storage,agent,manuscriptId);instances.push(execution);const file=await storage.files.readFile(manuscriptId,"main.tex");const context={manuscriptId,files:[{relativePath:"main.tex",expectedDiskSha256:file.sha256}]};const skill={type:"skill" as const,name:execution.resources.name,identity:{environment_id:"application",source_id:execution.resources.sourceId,revision:"1"}};return {execution,agent,context,skill};}
it("checks the exact admitted steer and structured input before handoff completion",async()=>{
 vi.useFakeTimers();const f=await fixture(),preview=await start(f),draft=insertAgentReference({text:"inspect",bindings:[]},f.skill);
 const intent:AgentSteeringHandoffIntent={operationId:crypto.randomUUID(),conversationId,turnId:preview.turnId,steeringInputId:crypto.randomUUID(),queueInputId:crypto.randomUUID(),
   expectedRevision:1,expectedEpoch:crypto.randomUUID(),scope:{kind:"manuscript",manuscriptId},queueIndex:0,queuePaused:false,inputPaused:false,origin:null,text:draft.text,userInput:agentDraftInput(draft)};
 expect(f.execution.steeringHandoffAdmission(intent)).toBe("absent");
 await f.execution.steer({conversationId,manuscriptId,turnId:intent.turnId,inputId:intent.steeringInputId,text:intent.text,userInput:intent.userInput});
 expect(f.execution.steeringHandoffAdmission(intent)).toBe("matching");
 expect(f.execution.steeringHandoffAdmission({...intent,userInput:[]})).toBe("conflicting");
 expect(f.execution.steeringHandoffAdmission({...intent,turnId:crypto.randomUUID()})).toBe("conflicting");
});
it("drops only unconsumed steer admissions on a simulated process reopen",async()=>{
 vi.useFakeTimers();const f=await fixture();f.execution.historyOnly=true;const preview=await start(f);
 const base={operationId:crypto.randomUUID(),conversationId,turnId:preview.turnId,queueInputId:crypto.randomUUID(),expectedRevision:1,expectedEpoch:crypto.randomUUID(),
   scope:{kind:"manuscript" as const,manuscriptId},queueIndex:0,queuePaused:false,inputPaused:false,origin:null,text:"Original",userInput:[]};
 const confirmed:AgentSteeringHandoffIntent={...base,steeringInputId:crypto.randomUUID()};
 await f.execution.steer({conversationId,manuscriptId,turnId:preview.turnId,inputId:confirmed.steeringInputId,text:confirmed.text,userInput:[]});
 f.execution.consumeSteersAtFixtureBoundary(conversationId,preview.turnId);
 const pending:AgentSteeringHandoffIntent={...base,operationId:crypto.randomUUID(),steeringInputId:crypto.randomUUID()};
 await f.execution.steer({conversationId,manuscriptId,turnId:preview.turnId,inputId:pending.steeringInputId,text:pending.text,userInput:[]});
 f.execution.restore(f.execution.snapshot());
 expect(f.execution.steeringHandoffAdmission(confirmed)).toBe("matching");
 expect(f.execution.steeringHandoffAdmission(pending)).toBe("absent");
 await vi.advanceTimersByTimeAsync(3000);
 expect(f.agent.history.steeringHandoffHistory(pending)).toBe("unconfirmed");
});
async function start(f:Awaited<ReturnType<typeof fixture>>,draft:AgentInputDraft={text:"start",bindings:[]}){const preview=await f.execution.submission.prepareWriting({conversationId,requestId:crypto.randomUUID(),text:draft.text,userInput:agentDraftInput(draft),context:f.context});const scope={manuscriptId,conversationId,requestId:preview.requestId,contextSnapshotId:preview.contextSnapshotId};await f.execution.workspace.prepare({scope,buffers:[]});await f.execution.submission.startWriting({...scope,model:preview.input.model,mode:"deepSeekNonThinking"});return preview;}
it("commits consecutive Steers at the model boundary, injects once and rejects identity reuse",async()=>{
 const f=await fixture();vi.useFakeTimers({toFake:["setTimeout","clearTimeout"]});const preview=await start(f);
 const draft=insertAgentReference({text:"use",bindings:[]},f.skill),first={manuscriptId,conversationId,turnId:preview.turnId,inputId:crypto.randomUUID(),text:draft.text,userInput:agentDraftInput(draft)};
 await f.execution.steer(first);await f.execution.steer(first);await f.execution.steer({...first,inputId:crypto.randomUUID()});await expect(f.execution.steer({...first,text:"different",userInput:[]})).rejects.toMatchObject({code:"conflict"});
 const read=()=>f.agent.gateways.delivery.sessions.readHistory({conversationId,afterSeq:0,limit:200});
 const before=await read();expect(before.events.filter(event=>event.payload.type==="itemStarted"&&event.payload.kind.type==="userMessage")).toHaveLength(1);expect(before.events.some(event=>event.payload.type==="turnFinished")).toBe(false);
 await vi.advanceTimersByTimeAsync(f.execution.delayMs);const after=await read();
 expect(after.events.filter(event=>event.payload.type==="itemStarted"&&event.payload.kind.type==="userMessage")).toHaveLength(3);
 const loads=after.events.filter(event=>event.payload.type==="itemStarted"&&event.payload.kind.type==="toolCall"&&event.payload.kind.name==="research_load_skill");expect(loads).toHaveLength(1);
 expect(after.events.some(event=>event.payload.type==="textDelta"&&event.payload.text.startsWith("已加载技能"))).toBe(false);
 expect(after.events.some(event=>event.payload.type==="textDelta"&&event.payload.text.includes('"name":"模拟写作技能"'))).toBe(true);
 expect(after.events.at(-1)?.payload.type).toBe("turnFinished");
});
it("retains exact references through queue edits and freezes selected skill content",async()=>{
 const f=await fixture(),queue=new WritingDiffDemoQueue(f.execution.submission, undefined, request => f.execution.materializeCompaction(request), scope => f.execution.inspectFrozen(scope),
   {validate: request => f.execution.validateQueuedStart(request), inspect: (request, preview) => f.execution.inspectStarted(request, preview)}),draft=insertAgentReference({text:"review",bindings:[]},f.skill),inputId=crypto.randomUUID();const added=await queue.gateway.edit({manuscriptId,conversationId,requestId:crypto.randomUUID(),expectedRevision:0,edit:{type:"add",inputId,text:draft.text,userInput:agentDraftInput(draft)}});
 const prepared=await queue.preview.prepare({inputId,expectedRevision:added.revision,submission:{conversationId,requestId:crypto.randomUUID(),text:draft.text,userInput:agentDraftInput(draft),context:f.context}});expect(prepared.preview.input.skillInstructions?.[0]?.contents).toContain("scripts/run.sh");expect(prepared.preview.input.userInput).toEqual(agentDraftInput(draft));
 f.execution.resources.revision="2";const unresolved=await f.execution.submission.prepareWriting({conversationId,requestId:crypto.randomUUID(),text:draft.text,userInput:agentDraftInput(draft),context:f.context});expect(unresolved.input.skillInstructions).toEqual([]);expect(unresolved.input.skills?.every(skill=>!skill.explicit)).toBe(true);expect((await queue.gateway.read({manuscriptId,conversationId})).inputs[0]?.userInput).toEqual(agentDraftInput(draft));
});
it("retains unavailable tool tags as text without expanding the active turn grants",async()=>{
 const f=await fixture();f.execution.resources.approved=false;const preview=await start(f);f.execution.resources.approved=true;const draft=insertAgentReference({text:"search",bindings:[]},{type:"mention",name:"模拟文献检索",target:{kind:"tool",serverId:"模拟文献检索",environmentId:"mock",revision:"1"}});
 await expect(f.execution.steer({manuscriptId,conversationId,turnId:preview.turnId,inputId:crypto.randomUUID(),text:draft.text,userInput:agentDraftInput(draft)})).resolves.toBeDefined();expect(f.execution.snapshot()[0]?.[1].toolsAllowed).toBe(false);
});

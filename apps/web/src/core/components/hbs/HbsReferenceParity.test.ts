import { afterEach, expect, it } from "vitest";
import { HbsSession, DocumentVersionConflict, createTextDocumentAdapter, type DocumentCheckpoint, type DocumentSnapshot } from "@research-assistant/core/hbs";
import traces from "../../../../../../tests/fixtures/hbs/hbs-reference-session-traces-baseline-2026-10-23.json";
import { directTextDiff } from "./codeMirrorHbsPolicy";
import { createCodeMirrorHbsPolicy } from "./codeMirrorHbsPolicy";
import { HbsClient, HbsCommitError, MemoryHbsPreservationStore, type HbsSnapshot, type HbsUnitUpdate } from "@research-assistant/core/hbs";

const sessions: HbsSession<string>[]=[];
afterEach(()=>sessions.splice(0).forEach(session=>session.dispose()));
const settle=async()=>{for(let n=0;n<80;n++)await Promise.resolve();};
/** Captured by executing the independent reference itself. No production or
 * test import depends on that checkout, its dependencies or its transport. */
it.each(traces.cases)("matches the independent reference: $name",async scenario=>{
  let server:DocumentSnapshot<string>={version:1,saved:traces.initial,baseline:traces.initial};
  let preserved:DocumentCheckpoint<string>|undefined;
  const options={adapter:createTextDocumentAdapter(directTextDiff),requestId:(n:number)=>`request-${n}`,
    preservation:{load:()=>preserved,save:(value:DocumentCheckpoint<string>)=>{preserved=structuredClone(value);}},
    port:{read:async()=>server,commit:async(request:{expectedVersion:number;saved:string;baseline:string})=>{
      if(request.expectedVersion!==server.version)throw new DocumentVersionConflict(server,"request");
      server={version:server.version+1,saved:request.saved,baseline:request.baseline};return server;
    }}};
  let session=await HbsSession.open(options);sessions.push(session);
  for(const [index,step] of scenario.steps.entries()){
    const [op,from,to,ordinary]=step;
    const state=session.getSnapshot();
    if(op==="external")server={...server,version:server.version+1,saved:server.saved.replace(String(from),String(to))};
    else if(op==="edit")session.edit(state.content.replace(String(from),String(to)),ordinary?state.baselineProjection.replace(String(from),String(to)):state.baselineProjection);
    else if(op==="sync")await session.syncLatest();
    else if(op==="save")await session.save();
    else if(op==="keep")session.review(state.content,state.baselineProjection.replace(String(from),String(to)));
    else if(op==="undo")session.review(state.content.replace(String(from),String(to)),state.baselineProjection);
    else if(op==="restart"){session.dispose();session=await HbsSession.open(options);sessions.push(session);}
    await settle();
    const current=session.getSnapshot();
    expect({h:current.content,b:current.baselineProjection,s:server.saved,confirmed:server.baseline,conflict:current.conflicts.length>0},`step ${index}: ${op}`).toEqual(scenario.states[index]);
  }
});

it.each(traces.cases)("preserves the existing text entry against the reference: $name",async scenario=>{
  const unit=(text:string)=>[{id:"note",label:"note",text}];
  let server:HbsSnapshot={version:1,saved:unit(traces.initial),baseline:unit(traces.initial)};
  const store=new MemoryHbsPreservationStore();
  const options=()=>({documentId:"note",policy:createCodeMirrorHbsPolicy(),preservation:store,requestId:(n:number)=>`request-${n}`,
    port:{initialSnapshot:server,subscribeSnapshots:()=>()=>{},syncLatest:async(known:number)=>known<server.version?server:undefined,
      commit:async(request:{id:string;expectedVersion:number;updates:readonly HbsUnitUpdate[]})=>{
        if(request.expectedVersion!==server.version)throw new HbsCommitError("MODEL_VERSION_CONFLICT",server);
        const update=request.updates[0]!;
        server={version:server.version+1,saved:unit(update.saved),baseline:unit(update.baseline)};
        return {requestId:request.id,version:server.version};
      }}});
  let client=await HbsClient.open(options());
  const h=()=>client.getSnapshot().draft?.value??client.getSnapshot().observed.saved[0]!.text;
  const b=()=>client.getSnapshot().draft?.editedBaseline??client.getSnapshot().observed.baseline[0]!.text;
  try{for(const [index,step] of scenario.steps.entries()){
    const [op,from,to,ordinary]=step;
    if(op==="external")server={...server,version:server.version+1,saved:unit(server.saved[0]!.text.replace(String(from),String(to)))};
    else if(op==="edit")client.edit(h().replace(String(from),String(to)),ordinary?b().replace(String(from),String(to)):b());
    else if(op==="sync")await client.syncLatest();
    else if(op==="save")client.save();
    else if(op==="keep"||op==="undo"){
      const value=op==="undo"?h().replace(String(from),String(to)):h(),baseline=op==="keep"?b().replace(String(from),String(to)):b();
      client.history.perform(()=>client.review(value,baseline),{value,baseline});
    }else if(op==="restart"){client.dispose();client=await HbsClient.open(options());}
    await settle();
    expect({h:h(),b:b(),s:server.saved[0]!.text,confirmed:server.baseline[0]!.text,conflict:client.getIncoming().length>0},`step ${index}: ${op}`).toEqual(scenario.states[index]);
  }}finally{client.dispose();}
});

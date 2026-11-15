import { afterEach, expect, it, vi } from "vitest";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { createWritingDiffDemoRuntime } from "./writingDiffDemoRuntime";
const manuscriptId='00000000-0000-4000-8000-000000000003',parentId='00000000-0000-4000-8000-000000000001';
const live: ReturnType<typeof createWritingDiffDemoRuntime>[]=[];
afterEach(()=>{for(const runtime of live.splice(0))runtime.dispose();});
it('copies only the selected terminal prefix, preserves parent history and retries the same branch identity',async()=>{
 const storage=new WritingDiffDemoStorage(manuscriptId,[]),agent=new WritingDiffDemoAgentStorage(storage,undefined);
 const runtime=createWritingDiffDemoRuntime(storage,agent,manuscriptId,parentId);live.push(runtime);
 const first={scope:{manuscriptId,conversationId:parentId,requestId:crypto.randomUUID(),contextSnapshotId:crypto.randomUUID()},turnId:crypto.randomUUID()};
 agent.history.start(first,'first');await agent.complete(first,[]);
 const second={scope:{...first.scope,requestId:crypto.randomUUID(),contextSnapshotId:crypto.randomUUID()},turnId:crypto.randomUUID()};
 agent.history.start(second,'second still running');
 const read=(conversationId:string)=>agent.history.gateway.readHistory({conversationId,afterSeq:0,limit:500});
 const parent=await read(parentId),request={childId:crypto.randomUUID(),parentId,throughTurnId:first.turnId,binding:{kind:'manuscript' as const,resourceId:manuscriptId}};
 const branch=await runtime.gateways.fork.fork(request);
 expect(await runtime.gateways.fork.fork(request)).toEqual(branch);
 expect(await read(parentId)).toEqual(parent);
 const child=await read(request.childId);
 expect(child.events.every(e=>e.conversationId===request.childId&&e.payload.turnId===first.turnId)).toBe(true);
 expect(child.events.at(-1)?.payload.type).toBe('turnFinished');
 expect(child.events.length).toBe(branch.parentEndSeq);
 await expect(runtime.gateways.fork.fork({...request,throughTurnId:second.turnId})).rejects.toMatchObject({code:'storage',reason:{code:'DATA_SQL_OPERATION_CONFLICT'}});
 await expect(runtime.gateways.fork.fork({...request,childId:crypto.randomUUID(),throughTurnId:second.turnId})).rejects.toBe('busy');
 expect((await runtime.gateways.forkOrigin.read({conversationId:request.childId,binding:request.binding})).origin?.throughTurnId).toBe(first.turnId);
});
it('keeps original fork proof across snapshot restore and later metadata changes; status never reads mutable history',async()=>{
 const storage=new WritingDiffDemoStorage(manuscriptId,[]),agent=new WritingDiffDemoAgentStorage(storage,undefined);
 const runtime=createWritingDiffDemoRuntime(storage,agent,manuscriptId,parentId);live.push(runtime);
 const input={scope:{manuscriptId,conversationId:parentId,requestId:crypto.randomUUID(),contextSnapshotId:crypto.randomUUID()},turnId:crypto.randomUUID()};
 agent.history.start(input,'synthetic');await agent.complete(input,[]);
 const request={childId:crypto.randomUUID(),parentId,throughTurnId:input.turnId,binding:{kind:'manuscript' as const,resourceId:manuscriptId}};
 const before=runtime.conversations.snapshot();
 expect((await runtime.gateways.fork.status(request)).status.type).toBe('noSqlReceipt');
 expect(runtime.conversations.snapshot()).toEqual(before);
 const branch=await runtime.gateways.fork.fork(request),proof=await runtime.gateways.fork.status(request);
 await runtime.conversations.catalog.rename({operationId:crypto.randomUUID(),id:request.childId,expectedTitle:branch.conversation.title,title:'Later name'});
 await runtime.conversations.archives.setArchived({operationId:crypto.randomUUID(),id:request.childId,archived:true,expectedWatermark:branch.conversation.durableCount});
 const saved=runtime.conversations.snapshot();runtime.conversations.restore(saved);
 const read=vi.spyOn(agent.history.gateway,'readHistory').mockRejectedValue('must not read mutable history');
 try { expect(await runtime.gateways.fork.status(request)).toEqual(proof);expect(read).not.toHaveBeenCalled(); }
 finally{read.mockRestore();}
 const replay=await runtime.gateways.fork.fork(request);expect(replay.conversation.title).toBe('Later name');expect(replay.conversation.archived).toBe(true);
 await expect(runtime.conversations.catalog.rename({operationId:request.childId,id:parentId,expectedTitle:'完善摘要',title:'Must not change'}))
  .rejects.toMatchObject({code:'storage',reason:{code:'DATA_SQL_OPERATION_CONFLICT'}});
 await expect(runtime.gateways.fork.status({...request,binding:{...request.binding,resourceId:'another'}}))
  .rejects.toMatchObject({code:'storage',reason:{code:'DATA_SQL_OPERATION_CONFLICT'}});
 expect(await runtime.gateways.fork.status(request)).toEqual(proof);
});
it('rejects a data epoch switched during asynchronous source reading before publishing any child or receipt',async()=>{
 const storage=new WritingDiffDemoStorage(manuscriptId,[]),agent=new WritingDiffDemoAgentStorage(storage,undefined);
 const runtime=createWritingDiffDemoRuntime(storage,agent,manuscriptId,parentId);live.push(runtime);
 const input={scope:{manuscriptId,conversationId:parentId,requestId:crypto.randomUUID(),contextSnapshotId:crypto.randomUUID()},turnId:crypto.randomUUID()};
 agent.history.start(input,'synthetic');await agent.complete(input,[]);
 const request={childId:crypto.randomUUID(),parentId,throughTurnId:input.turnId,binding:{kind:'manuscript' as const,resourceId:manuscriptId}};
 const saved=runtime.conversations.snapshot(),oldEpoch=saved.renames.epoch,newEpoch=crypto.randomUUID();
 const pending=runtime.gateways.fork.fork(request,{expectedEpoch:oldEpoch});
 runtime.conversations.restore({...saved,renames:{...saved.renames,epoch:newEpoch},
  archiveReceipts:{...saved.archiveReceipts,epoch:newEpoch},forkReceipts:{...saved.forkReceipts,epoch:newEpoch},deletionReceipts:{...saved.deletionReceipts,epoch:newEpoch}});
 await expect(pending).rejects.toMatchObject({code:'storage',reason:{code:'DATA_SQL_OPERATION_CONFLICT',reason:{safeCause:'data_epoch_changed'}}});
 expect(runtime.conversations.snapshot().records.has(request.childId)).toBe(false);
 expect(agent.history.snapshot().events.has(request.childId)).toBe(false);
 expect((await runtime.gateways.fork.status(request)).status).toMatchObject({type:'noSqlReceipt',epoch:newEpoch});
 await runtime.gateways.fork.fork(request,{expectedEpoch:newEpoch});
 await expect(runtime.gateways.fork.fork(request,{expectedEpoch:oldEpoch})).rejects.toMatchObject({code:'storage',reason:{code:'DATA_SQL_OPERATION_CONFLICT'}});
});

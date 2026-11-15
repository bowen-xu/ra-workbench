import {afterEach,expect,it,vi} from "vitest";
import type {AgentNoteQueueScope,AgentNoteQueuePrepareRequest,AgentNoteQueuePreview,AgentNoteQueueAdmission,AgentStartResponse} from "@research-assistant/core/agent";
import {NoteQueueSubmissionController,noteQueueSubmissionOwnerKey,noteQueueSubmissionOwners,resetNoteQueueSubmissionOwnersForTests} from "./noteQueueSubmissionController";
const conversationId="a1111111-1111-4111-8111-111111111111",resourceId="b1111111-1111-4111-8111-111111111111";
const requestId="c1111111-1111-4111-8111-111111111111",inputId="d1111111-1111-4111-8111-111111111111";
const turnId="e1111111-1111-4111-8111-111111111111",contextSnapshotId="f1111111-1111-4111-8111-111111111111";
function fixture(kind:AgentNoteQueueScope["scope"]["kind"]="note"){
  const scope:AgentNoteQueueScope={conversationId,scope:{kind,resourceId}};
  const base={conversationId,requestId,text:"First queued message"};
  const request:AgentNoteQueuePrepareRequest={inputId,expectedRevision:1,submission:kind==="note"
    ?{kind,request:{...base,context:{pageId:resourceId,expectedRevision:2,blockId:null}}}
    :{kind,request:{...base,context:{readingNoteId:resourceId,expectedRevision:2}}}};
  const preview:AgentNoteQueuePreview={schemaVersion:1,inputId,scope:scope.scope,
    record:{conversationId,requestId,turnId,contextSnapshotId,input:{text:base.text,model:{providerId:"deepseek",modelId:"deepseek-v4-pro",revision:null},reasoningEffort:null,context:[]}},
    queue:{schemaVersion:1,...scope,revision:2,paused:false,preparingInputId:inputId,inputs:[{inputId,text:base.text}]}};
  const response:AgentStartResponse={schemaVersion:1,conversationId,requestId,turnId,alreadyStarted:false,outcome:null};
  const admission:AgentNoteQueueAdmission={requestId,turnId,createdAt:"2026-09-11T12:00:00Z",messageExcerpt:base.text,queue:{inputId,expectedRevision:2}};
  const gateway={read:vi.fn(),edit:vi.fn(),prepare:vi.fn().mockResolvedValue(preview),recover:vi.fn().mockResolvedValue(preview),
    cancel:vi.fn().mockResolvedValue({...preview.queue,revision:3,paused:true,preparingInputId:null}),start:vi.fn().mockResolvedValue(response),
    restore:vi.fn().mockResolvedValue(preview.record),list:vi.fn()};
  const turns={interrupt:vi.fn().mockResolvedValue({schemaVersion:1,conversationId,turnId,cancellationRequested:true})};
  const controller=new NoteQueueSubmissionController(scope,gateway,turns);
  return {scope,request,preview,response,admission,gateway,turns,controller};
}
afterEach(()=>resetNoteQueueSubmissionOwnersForTests());
for(const kind of ["note","readingNote"] as const){
  it(`reviews ${kind} head then starts only its frozen confirmation`,async()=>{
    const f=fixture(kind);await f.controller.prepare(f.request);
    expect(f.gateway.start).not.toHaveBeenCalled();expect(f.controller.dismiss()).toBe(false);
    Object.assign(f.request.submission.request,{text:"Later caller message"});
    await f.controller.confirm("deepSeekNonThinking");
    expect(f.gateway.start).toHaveBeenCalledExactlyOnceWith(f.scope,f.preview.record,{inputId,expectedRevision:2},"deepSeekNonThinking");
    expect(f.controller.getSnapshot().request?.submission.request.text).toBe("First queued message");
    await f.controller.stop();await f.controller.stop();expect(f.turns.interrupt).toHaveBeenCalledOnce();
    f.controller.observeOutcome(turnId,"canceled");expect(f.controller.dismiss()).toBe(true);
  });
  it(`restores admitted ${kind} with its old queue revision without preparing current evidence`,async()=>{
    const f=fixture(kind);await f.controller.recoverAdmission(f.admission);
    expect(f.gateway.prepare).not.toHaveBeenCalled();expect(f.gateway.recover).not.toHaveBeenCalled();
    expect(f.gateway.restore).toHaveBeenCalledExactlyOnceWith(f.scope,requestId);
    Object.assign(f.admission.queue,{expectedRevision:99});
    await f.controller.confirm("deepSeekNonThinking");
    expect(f.gateway.start.mock.calls[0]?.[2]).toEqual({inputId,expectedRevision:2});
  });
}
it("holds unknown preparation to the same request and refuses another pending head",async()=>{
  const f=fixture();f.gateway.prepare.mockRejectedValueOnce(new Error("lost reply"));
  await f.controller.prepare(f.request);expect(f.controller.dismiss()).toBe(false);
  f.gateway.recover.mockResolvedValueOnce({...f.preview,inputId:turnId});
  await f.controller.retryPrepare(true);expect(f.controller.getSnapshot().error).toBe("prepare");
  await f.controller.retryPrepare();expect(f.gateway.prepare.mock.calls[0]).toEqual(f.gateway.prepare.mock.calls[1]);
  expect(f.controller.getSnapshot().phase).toBe("preview");
});
it("keeps unknown cancellation on its exact receipt even after a later rejection",async()=>{
  const f=fixture();await f.controller.prepare(f.request);
  f.gateway.cancel.mockRejectedValueOnce(new Error("lost reply")).mockRejectedValueOnce({code:"queue",reason:{code:"closed"}});
  expect(await f.controller.cancel(contextSnapshotId)).toBe(false);
  expect(f.controller.getSnapshot().cancelError).toBe("uncertain");
  await f.controller.confirm("deepSeekNonThinking");await f.controller.refreshPending();expect(f.controller.dismiss()).toBe(false);
  expect(f.gateway.start).not.toHaveBeenCalled();expect(f.gateway.recover).not.toHaveBeenCalled();
  await f.controller.cancel(turnId);expect(f.controller.getSnapshot().cancelError).toBe("uncertain");
  await f.controller.cancel(resourceId);
  expect(f.gateway.cancel.mock.calls[0]).toEqual(f.gateway.cancel.mock.calls[1]);
  expect(f.gateway.cancel.mock.calls[1]).toEqual(f.gateway.cancel.mock.calls[2]);
  expect(f.controller.getSnapshot().phase).toBe("idle");expect(f.controller.getSnapshot().cancelError).toBeNull();
});

it("reattaches a switched queue view to the same owner while head preparation is pending",async()=>{
  const f=fixture();let resolve!:(value:AgentNoteQueuePreview)=>void;
  f.gateway.prepare.mockImplementationOnce(()=>new Promise<AgentNoteQueuePreview>(done=>{resolve=done;}));
  const key=noteQueueSubmissionOwnerKey(f.scope);
  const first=noteQueueSubmissionOwners.acquire(key,()=>new NoteQueueSubmissionController(f.scope,f.gateway,f.turns));
  const pending=first.controller.prepare(f.request);first.release();
  const second=noteQueueSubmissionOwners.acquire(key,()=>new NoteQueueSubmissionController(f.scope,f.gateway,f.turns));
  expect(second.controller).toBe(first.controller);
  resolve(f.preview);await pending;
  expect(second.controller.getSnapshot()).toMatchObject({phase:"preview",request:f.request});second.release();
});
it("refreshes a definitively stale preview to its original head at the latest revision",async()=>{
  const f=fixture();await f.controller.prepare(f.request);
  f.gateway.cancel.mockRejectedValueOnce({code:"queue",reason:{code:"conflict"}});
  await f.controller.cancel(contextSnapshotId);expect(f.controller.getSnapshot().cancelError).toBe("rejected");
  f.gateway.recover.mockResolvedValue({...f.preview,queue:{...f.preview.queue,revision:7}});
  await f.controller.refreshPending();expect(f.controller.getSnapshot().preview?.confirmation.expectedRevision).toBe(7);
  await f.controller.cancel(resourceId);
  expect(f.gateway.cancel.mock.calls[1]?.[0]).toEqual({...f.scope,requestId:resourceId,previewRequestId:requestId,inputId,expectedRevision:7});
});
it("distinguishes first queue rejection from a rejection following an unknown start",async()=>{
  const clean=fixture();await clean.controller.prepare(clean.request);
  clean.gateway.start.mockRejectedValueOnce({code:"queue",reason:{code:"conflict"}});
  await clean.controller.confirm("deepSeekNonThinking");expect(clean.controller.getSnapshot().error).toBe("startRejected");
  await clean.controller.refreshPending();expect(clean.controller.getSnapshot().phase).toBe("preview");
  const uncertain=fixture();await uncertain.controller.prepare(uncertain.request);
  uncertain.gateway.start.mockRejectedValueOnce(new Error("lost reply")).mockRejectedValueOnce({code:"queue",reason:{code:"conflict"}});
  await uncertain.controller.confirm("deepSeekNonThinking");await uncertain.controller.confirm("deepSeekNonThinking");
  expect(uncertain.controller.getSnapshot().error).toBe("start");
  expect(await uncertain.controller.cancel(contextSnapshotId)).toBe(false);
  await uncertain.controller.refreshPending();expect(uncertain.gateway.recover).not.toHaveBeenCalled();
  expect(uncertain.gateway.start.mock.calls[0]).toEqual(uncertain.gateway.start.mock.calls[1]);
});
it("keeps an observed durable outcome when start later reports an unknown result",async()=>{
  const f=fixture();await f.controller.prepare(f.request);let reject!:(error:unknown)=>void;
  f.gateway.start.mockImplementationOnce(()=>new Promise((_done,fail)=>{reject=fail;}));
  const pending=f.controller.confirm("deepSeekNonThinking");f.controller.observeOutcome(turnId,"completed");
  reject(new Error("response lost"));await pending;
  expect(f.controller.getSnapshot().outcome).toBe("completed");expect(f.controller.dismiss()).toBe(true);
});
it("fences disposed reads and isolates a mismatched host before preparation",async()=>{
  const f=fixture();let finish!:(value:unknown)=>void;
  f.gateway.prepare.mockImplementationOnce(()=>new Promise(done=>{finish=done;}));
  const pending=f.controller.prepare(f.request);f.controller.dispose();finish(f.preview);await pending;
  expect(f.controller.getSnapshot().preview).toBeNull();expect(f.gateway.start).not.toHaveBeenCalled();
  const g=fixture("readingNote");await expect(g.controller.prepare(f.request)).rejects.toThrow();expect(g.gateway.prepare).not.toHaveBeenCalled();
});

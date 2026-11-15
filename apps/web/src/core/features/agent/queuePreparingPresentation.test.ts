import { describe, expect, it } from "vitest";
import { independentQueuePreparingPresentation, writingQueuePreparingPresentation } from "./queuePreparingPresentation";

describe("queue preparation presentation", () => {
  const preparation = {phase:"sending", recovered:false, intent:{inputId:"input"}};
  const request = {conversationId:"conversation",requestId:"request"};
  const receipt = {...request,turnId:"turn"};
  const start = {phase:"sending", recovered:false, turnId:"turn", intent:{inputId:"input",start:request}, result:null};
  it("recognizes only the original live writing request and its confirmed start", () => {
    expect(writingQueuePreparingPresentation(null, null)).toBe("none");
    expect(writingQueuePreparingPresentation("input", {phase:"preparing",error:null,preparation})).toBe("sending");
    expect(writingQueuePreparingPresentation("input", {phase:"starting",error:null,startRecovery:start})).toBe("sending");
    expect(writingQueuePreparingPresentation("input", {phase:"started",error:null,startRecovery:{...start,phase:"done",result:receipt}})).toBe("sent");
    expect(writingQueuePreparingPresentation("input", {phase:"started",error:"start",startRecovery:{...start,phase:"localError",result:receipt}})).toBe("recovery");
    expect(writingQueuePreparingPresentation("input", {phase:"started",error:null,startRecovery:{...start,phase:"done",result:{...receipt,turnId:"different"}}})).toBe("recovery");
    expect(writingQueuePreparingPresentation("other", {phase:"preparing",error:null,preparation})).toBe("recovery");
    expect(writingQueuePreparingPresentation("input", {phase:"error",error:"prepare",preparation})).toBe("recovery");
    expect(writingQueuePreparingPresentation("input", {phase:"preparing",error:null,preparation:{...preparation,recovered:true}})).toBe("recovery");
    expect(writingQueuePreparingPresentation("input", {phase:"starting",error:null,startRecovery:{...start,phase:"unknown"}})).toBe("recovery");
    expect(writingQueuePreparingPresentation("input", null)).toBe("recovery");
  });
  it("keeps the active queue input sending across workspace preparation and rejects recovered or unrelated claims", () => {
    for (const phase of ["ready", "persisting", "clearing", "complete"]) {
      const workspacePreparation = {...preparation, phase};
      expect(writingQueuePreparingPresentation("input", {phase:"preview",error:null,preparation:workspacePreparation})).toBe("sending");
      expect(writingQueuePreparingPresentation("other", {phase:"preview",error:null,preparation:workspacePreparation})).toBe("recovery");
      expect(writingQueuePreparingPresentation("input", {phase:"preview",error:null,preparation:{...workspacePreparation,recovered:true}})).toBe("recovery");
      expect(writingQueuePreparingPresentation("input", {phase:"preview",error:"workspace",preparation:workspacePreparation})).toBe("recovery");
    }
    expect(writingQueuePreparingPresentation("input", {phase:"preview",error:null,preparation:{...preparation,phase:"unknown"}})).toBe("recovery");
    expect(writingQueuePreparingPresentation("input", null, "input")).toBe("sending");
    expect(writingQueuePreparingPresentation("input", {phase:"idle",error:null}, "input")).toBe("sending");
    expect(writingQueuePreparingPresentation("input", {phase:"error",error:"prepare"}, "input")).toBe("recovery");
    expect(writingQueuePreparingPresentation("input", {phase:"idle",error:null,preparation}, "input")).toBe("recovery");
    expect(writingQueuePreparingPresentation("input", null, "other")).toBe("recovery");
  });
  it("keeps independent recovered, unknown and unrelated claims available for recovery", () => {
    const owner = {phase:"sending",record:{intent:{queue:{inputId:"input"}},committed:null,start:null}};
    expect(independentQueuePreparingPresentation("input",owner,true)).toBe("sending");
    expect(independentQueuePreparingPresentation("input",owner,false)).toBe("recovery");
    expect(independentQueuePreparingPresentation("other",owner,true)).toBe("recovery");
    expect(independentQueuePreparingPresentation("input",{...owner,phase:"unknown"},true)).toBe("recovery");
    expect(independentQueuePreparingPresentation("input",{...owner,phase:"ready"},true,"input")).toBe("sending");
    expect(independentQueuePreparingPresentation("input",{...owner,phase:"ready"},false,"input")).toBe("recovery");
    expect(independentQueuePreparingPresentation("input",{...owner,phase:"ready"},true,"other")).toBe("recovery");
    expect(independentQueuePreparingPresentation("input",{...owner,phase:"ready"},true)).toBe("recovery");
    expect(independentQueuePreparingPresentation("other",{...owner,phase:"ready"},true,"other")).toBe("recovery");
    expect(independentQueuePreparingPresentation("input",{...owner,phase:"unknown"},true,"input")).toBe("recovery");
    expect(independentQueuePreparingPresentation("input",undefined,true,"input")).toBe("sending");
    expect(independentQueuePreparingPresentation("input",{phase:"idle",record:null},true,"input")).toBe("sending");
    expect(independentQueuePreparingPresentation("input",{phase:"unknown",record:null},true,"input")).toBe("recovery");
    expect(independentQueuePreparingPresentation("input",undefined,true,"other")).toBe("recovery");
    const independentRequest = {...request,queue:{inputId:"input"}};
    expect(independentQueuePreparingPresentation("input",{...owner,phase:"started",record:{...owner.record,committed:{result:{turnId:"turn"}},start:{request:independentRequest,result:receipt}}},false)).toBe("sent");
    expect(independentQueuePreparingPresentation("input",{...owner,phase:"started",record:{...owner.record,committed:{result:{turnId:"other"}},start:{request:independentRequest,result:receipt}}},false)).toBe("recovery");
    expect(independentQueuePreparingPresentation("input",undefined,false)).toBe("recovery");
  });
});

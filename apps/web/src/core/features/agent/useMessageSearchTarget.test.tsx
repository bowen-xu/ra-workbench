/** @vitest-environment happy-dom */
import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useMessageSearchTarget, type AgentMessageSearchTarget } from "./useMessageSearchTarget";

const rows=(...ids:string[])=>ids.map(id=>({id,turnId:id.split("/")[0]!}));

it.each(["message", "turn"] as const)("retains a hidden %s target and invalidates a removed identity without scrolling updates", async kind => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host=document.createElement("div");document.body.append(host);
  const root=createRoot(host), preserve=vi.fn();
  host.addEventListener("writing-agent-preserve-position",preserve,true);
  const focus=vi.spyOn(HTMLElement.prototype,"focus");
  const target:AgentMessageSearchTarget=kind==="message"?{requestId:1,messageId:"removed/item"}:{requestId:1,turnId:"removed"};
  let result:ReturnType<typeof useMessageSearchTarget>;
  function Probe({messages,ready=true,hidden=false}:{messages:readonly {id:string;turnId:string}[];ready?:boolean;hidden?:boolean}) {
    const container=useRef<HTMLDivElement>(null);
    result=useMessageSearchTarget(target,true,container,messages,ready,false);
    return <div ref={container}>{!hidden&&messages.map(message=><article key={message.id} tabIndex={-1} data-agent-message-id={message.id} data-agent-turn-id={message.turnId} />)}</div>;
  }
  try {
    await act(async()=>root.render(<Probe messages={rows("removed/item")}/>));
    expect(result!.matchedId).toBe("removed/item");
    expect(result!.feedback).toBeNull();
    expect(preserve).toHaveBeenCalledTimes(1);
    expect(focus).toHaveBeenCalledTimes(1);
    await act(async()=>root.render(<Probe messages={rows("removed/item","later/item")}/>));
    expect(preserve).toHaveBeenCalledTimes(1);
    expect(focus).toHaveBeenCalledTimes(1);
    await act(async()=>root.render(<Probe messages={rows("removed/item","later/item")} hidden/>));
    expect(result!.matchedId).toBe("removed/item");
    expect(result!.feedback).toBeNull();
    await act(async()=>root.render(<Probe messages={rows("removed/item","later/item","new/item")} hidden/>));
    expect(result!.matchedId).toBe("removed/item");
    expect(result!.feedback).toBeNull();
    await act(async()=>root.render(<Probe messages={rows("removed/item","later/item","new/item")}/>));
    expect(preserve).toHaveBeenCalledTimes(1);
    expect(focus).toHaveBeenCalledTimes(1);
    await act(async()=>root.render(<Probe messages={rows("later/item")} ready={false}/>));
    expect(result!.matchedId).toBe("removed/item");
    await act(async()=>root.render(<Probe messages={rows("later/item")}/>));
    expect(result!.matchedId).toBeUndefined();
    expect(result!.feedback).toContain(kind==="message"?"未找到原搜索消息":"未找到分支来源轮次");
    await act(async()=>root.render(<Probe messages={rows("later/item","new/item")}/>));
    expect(result!.matchedId).toBeUndefined();
    expect(preserve).toHaveBeenCalledTimes(1);
    expect(focus).toHaveBeenCalledTimes(1);
  } finally {
    await act(async()=>root.unmount());host.remove();focus.mockRestore();vi.unstubAllGlobals();
  }
});

it("retains a located compact turn activity with no messages while its element remains", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const host=document.createElement("div"),root=createRoot(host);
  const target={requestId:1,turnId:"compact"};
  let result:ReturnType<typeof useMessageSearchTarget>;
  function Probe({messages}:{messages:readonly {id:string;turnId:string}[]}) {
    const container=useRef<HTMLDivElement>(null);
    result=useMessageSearchTarget(target,true,container,messages,true,false);
    return <div ref={container}><div data-agent-turn-id="compact" tabIndex={-1}/></div>;
  }
  try {
    await act(async()=>root.render(<Probe messages={[]}/>));
    expect(result!.feedback).toBeNull();
    await act(async()=>root.render(<Probe messages={rows("later/item")}/>));
    expect(result!.feedback).toBeNull();
  } finally { await act(async()=>root.unmount());vi.unstubAllGlobals(); }
});

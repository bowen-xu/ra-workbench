/** @vitest-environment happy-dom */
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useAgentCommandExecution } from "./useAgentCommandExecution";
import { dispatchQueuedAgentCommand } from "./dispatchQueuedAgentCommand";
import { AgentCompactionActivity } from "./AgentCompactionActivity";
import { notify } from "../../services/notifications";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
it("retries only queue removal after successful compaction, without another provider request or clearing the draft", async () => {
  const result = {beforeTokens:82000,usage:{inputTokens:18000,contextWindowTokens:128000}};
  const gateway = {readUsage:vi.fn(async()=>result.usage),compact:vi.fn(async()=>result)};
  const remove = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true), release = vi.fn();
  const error = vi.spyOn(notify,"error").mockReturnValue(1);
  function Harness() {
    const [text,change]=useState("新草稿");
    const execution=useAgentCommandExecution({gateway,conversationId:"conversation",text,change});
    return <><AgentCompactionActivity records={execution.activities}/><output>{text}</output>
      <button onClick={()=>void dispatchQueuedAgentCommand({inputId:"queued-command",text:"/compact",running:false,execution,transfer:()=>({apply:remove,release})})}>执行队列命令</button></>;
  }
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  try {
    await act(async()=>root.render(<Harness/>));
    await act(async()=>host.querySelector("button")!.click());
    expect(error).toHaveBeenCalledWith("上下文已压缩，但队列项移除未确认，请先核对队列，勿重复执行。");
    await act(async()=>host.querySelector("button")!.click());
    expect(gateway.compact).toHaveBeenCalledOnce();
    expect(gateway.compact.mock.calls[0]).toEqual([{conversationId:"conversation",requestId:"queued-command",signal:expect.any(AbortSignal)}]);
    expect(remove).toHaveBeenCalledTimes(2);expect(release).toHaveBeenCalledTimes(2);
    expect(host.querySelector("output")!.textContent).toBe("新草稿");
    expect(host.querySelectorAll(".agent-compaction-activity")).toHaveLength(1);
    expect(host.querySelector(".agent-compaction-activity")!.textContent).toBe("上下文已压缩");
  } finally {await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();}
});

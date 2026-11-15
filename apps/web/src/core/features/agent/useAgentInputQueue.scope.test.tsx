/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import type {AgentQueueState} from "@research-assistant/core/agent";
import type {AgentQueueGateway} from "@research-assistant/data/agent";
import {notify} from "../../services/notifications";
import {AgentInputQueue} from "./AgentInputQueue";
import {useAgentInputQueue} from "./useAgentInputQueue";

vi.mock("../../services/notifications", () => ({notify: {error: vi.fn(), success: vi.fn()}}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT: boolean}).IS_REACT_ACT_ENVIRONMENT = true;

it("ignores a detached queue read failure but reports a current queue failure", async () => {
  const first = {conversationId:"11111111-1111-4111-8111-111111111111"};
  const second = {conversationId:"22222222-2222-4222-8222-222222222222"};
  const fail = new Map<string, (reason:unknown) => void>();
  const gateway:AgentQueueGateway<typeof first, AgentQueueState> = {
    read: scope => new Promise((_resolve, reject) => {fail.set(scope.conversationId, reject);}),
    edit: async () => {throw new Error("unexpected edit");},
  };
  const rendered: Array<{id:string; failure:unknown}> = [];
  function View({scope}:{scope:typeof first}) {
    const {state} = useAgentInputQueue(gateway, scope, true, 0);
    rendered.push({id:scope.conversationId, failure:state?.readFailure});
    return state ? <AgentInputQueue state={state} disabled={false} onEdit={async () => false}
      onReload={() => undefined} onRetry={() => undefined}/> : null;
  }
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<View scope={first}/>));
    await act(async () => root.render(<View scope={second}/>));
    await act(async () => fail.get(first.conversationId)!({code:"closed"}));
    expect(notify.error).not.toHaveBeenCalled();
    expect(rendered.filter(item => item.id === second.conversationId).every(item => !item.failure)).toBe(true);
    await act(async () => fail.get(second.conversationId)!({code:"closed"}));
    expect(notify.error).toHaveBeenCalledExactlyOnceWith("读取队列失败，请重试。", expect.any(Object));
  } finally {await act(async () => root.unmount()); host.remove(); vi.clearAllMocks();}
});

it("never renders a previous conversation's queue error under the new scope", async () => {
  const first = {conversationId:"11111111-1111-4111-8111-111111111111"};
  const second = {conversationId:"22222222-2222-4222-8222-222222222222"};
  const gateway:AgentQueueGateway<typeof first, AgentQueueState> = {
    read: scope => scope.conversationId === first.conversationId ? Promise.reject({code:"closed"}) : new Promise(() => undefined),
    edit: async () => {throw new Error("unexpected edit");},
  };
  const rendered: Array<{id:string; failure:unknown}> = [];
  function View({scope}:{scope:typeof first}) {
    const {state} = useAgentInputQueue(gateway, scope, true, 0);
    rendered.push({id:scope.conversationId, failure:state?.readFailure});
    return null;
  }
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<View scope={first}/>));
    expect(rendered.some(item => item.id === first.conversationId && item.failure)).toBe(true);
    await act(async () => root.render(<View scope={second}/>));
    expect(rendered.filter(item => item.id === second.conversationId).every(item => !item.failure)).toBe(true);
  } finally {await act(async () => root.unmount()); host.remove();}
});

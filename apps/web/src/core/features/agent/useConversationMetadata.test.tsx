/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AgentConversationSummary } from "@research-assistant/core/agent";
import { useConversationMetadata } from "./useConversationMetadata";
vi.mock("../../services/agentGateway", () => ({ agentMetadataGateway: {} }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const summary = (id: string, archived = false): AgentConversationSummary => ({ id, title: id, archived, durableCount: 0, createdAt: "now", updatedAt: "now" });
it("refreshes exact metadata, blocks on failure and rejects old responses across scopes and retries", async () => {
  const requests: { id: string; resolve: (value: AgentConversationSummary) => void; reject: (error: unknown) => void }[] = [];
  const gateway = { read: vi.fn((request) => new Promise<AgentConversationSummary>((resolve, reject) => requests.push({ id: request.id, resolve, reject }))) };
  let state!: ReturnType<typeof useConversationMetadata>;
  function Probe({ id, host }: { id: string; host: string }) {
    state = useConversationMetadata(id, { kind: "manuscript", resourceId: host }, gateway); return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<StrictMode><Probe id="a" host="one" /></StrictMode>));
    expect(state.status).toBe("loading"); const oldRefresh = state.refresh;
    await act(async () => requests.at(-1)!.resolve(summary("a")));
    expect(state.status).toBe("ready");
    await act(async () => { void state.refresh(); });
    expect(state.status).toBe("loading"); expect(state.unavailableReason).toBeDefined();
    await act(async () => requests.at(-1)!.resolve(summary("a", true)));
    expect(state.conversation?.archived).toBe(true);
    await act(async () => { void state.refresh(); });
    await act(async () => requests.at(-1)!.reject("notFound"));
    expect(state.status).toBe("error"); expect(state.unavailableReason).toContain("不再属于");
    await act(async () => { void state.refresh(); }); const late = requests.at(-1)!;
    await act(async () => root.render(<StrictMode><Probe id="b" host="two" /></StrictMode>));
    expect(gateway.read).toHaveBeenLastCalledWith({ id: "b", binding: { kind: "manuscript", resourceId: "two" } });
    await act(async () => requests.at(-1)!.resolve(summary("b")));
    const count = requests.length;
    await act(async () => oldRefresh()); expect(requests).toHaveLength(count);
    await act(async () => late.resolve(summary("a", true)));
    expect(state.conversation?.id).toBe("b"); expect(state.conversation?.archived).toBe(false);
    await act(async () => requests[0]!.resolve(summary("a", true)));
    expect(state.conversation?.id).toBe("b");
  } finally { await act(async () => root.unmount()); }
});

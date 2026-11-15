/** @vitest-environment happy-dom */
import { act, useLayoutEffect } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AgentConversationSnapshot } from "./conversationController";
import { useAgentConversation } from "./useAgentConversation";
import { notify } from "../../services/notifications";

const controls = vi.hoisted(() => ({ publish: [] as ((snapshot: AgentConversationSnapshot) => void)[] }));
vi.mock("../../services/agentGateway", () => ({ agentSessionGateway: {} }));
vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn() } }));
vi.mock("./conversationController", () => ({ AgentConversationController: class {
  snapshot: AgentConversationSnapshot = { status: "idle", projection: null, error: null };
  listener = () => {};
  constructor() { controls.publish.push((next) => { this.snapshot = next; this.listener(); }); }
  getSnapshot = () => this.snapshot;
  subscribe(listener: () => void) { this.listener = listener; return () => { this.listener = () => {}; }; }
  open() {}
  dispose() {}
} }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("isolates repeated history activations before layout navigation and exposes read failures", async () => {
  const observed: string[] = [];
  function Probe({ activation }: { activation: object }) {
    const snapshot = useAgentConversation("conversation", activation);
    useLayoutEffect(() => { observed.push(snapshot.status); });
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Probe activation={{}} />));
    const first = controls.publish.at(-1)!;
    await act(async () => first({ status: "ready", projection: null, error: null }));
    expect(observed.at(-1)).toBe("ready");
    observed.length = 0;
    await act(async () => root.render(<Probe activation={{}} />));
    expect(observed).toEqual(["idle"]);
    await act(async () => first({ status: "ready", projection: null, error: null }));
    expect(observed).toEqual(["idle"]);
    await act(async () => controls.publish.at(-1)!({ status: "error", projection: null, error: new Error("read") }));
    expect(observed.at(-1)).toBe("error");
  } finally { await act(async () => root.unmount()); }
});

it("reports the retained legacy import through the production history notification", async () => {
  vi.mocked(notify.error).mockClear();
  function Probe() { useAgentConversation("conversation"); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Probe />));
    await act(async () => controls.publish.at(-1)!({ status: "error", projection: null,
      error: {code: "agent_legacy_migration_incomplete", message: "private"} }));
    expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("agent_legacy_migration_incomplete; conversation=conversation"));
    expect(vi.mocked(notify.error).mock.calls[0]![0]).toContain("原历史已保留");
    expect(vi.mocked(notify.error).mock.calls[0]![0]).not.toContain("private");
  } finally { await act(async () => root.unmount()); }
});

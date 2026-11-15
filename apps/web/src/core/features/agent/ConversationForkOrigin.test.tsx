/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { ConversationForkOrigin } from "./ConversationForkOrigin";
import type { AgentForkOriginGateway } from "@research-assistant/data/agent";
import { copyText } from "./copyText";
import fixture from "../../../../../../tests/fixtures/agent/conversation-fork-origin.json";
vi.mock("./copyText", () => ({copyText: vi.fn().mockResolvedValue(true)}));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
it("distinguishes roots, unavailable lineage and archived parents; navigation is explicit", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const failure = Object.assign(new Error("permission denied: private request body"), {code: "FORK_READ_DENIED", requestId: "request-1", status: 403});
  const read = vi.fn<AgentForkOriginGateway["read"]>().mockRejectedValueOnce(failure).mockResolvedValueOnce({ ...fixture, schemaVersion: 1 });
  const onOpen = vi.fn();
  try {
    await act(async () => root.render(<ConversationForkOrigin conversationId={fixture.conversationId} binding={{ kind: "note", resourceId: "note" }} onOpen={onOpen} gateway={{ read }} />));
    expect(host.textContent).toContain("无法读取分支来源");
    expect(host.textContent).not.toContain("FORK_READ_DENIED");
    expect(host.textContent).not.toContain("private request body");
    expect(host.querySelector('[role="alert"]')?.classList.contains("failure")).toBe(true);
    expect(onOpen).not.toHaveBeenCalled();
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="复制诊断信息"]')!.click());
    const diagnostic = vi.mocked(copyText).mock.calls.at(-1)![0];
    expect(diagnostic).toContain("FORK_READ_DENIED");
    expect(diagnostic).toContain("request-1");
    expect(diagnostic).toContain("HTTP 403");
    expect(diagnostic).toContain("permission_denied");
    expect(diagnostic).not.toContain("private request body");
    await act(async () => host.querySelector("button")!.click());
    expect(read.mock.calls[0]).toEqual(read.mock.calls[1]); expect(host.textContent).toContain("已归档");
    expect(onOpen).not.toHaveBeenCalled();
    await act(async () => host.querySelector("button")!.click());
    expect(onOpen).toHaveBeenCalledWith(fixture.origin.parent, fixture.origin.throughTurnId);
    read.mockResolvedValue({ schemaVersion: 1, conversationId: fixture.conversationId, origin: null });
    await act(async () => root.render(<ConversationForkOrigin key="root" conversationId={fixture.conversationId} binding={{ kind: "note", resourceId: "note" }} onOpen={onOpen} gateway={{ read }} />));
    expect(host.textContent).toBe(""); expect(onOpen).toHaveBeenCalledOnce();
  } finally { await act(async () => root.unmount()); host.remove(); }
});
it("drops the old host response without showing or opening its parent", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let finish!: (value: Awaited<ReturnType<AgentForkOriginGateway["read"]>>) => void;
  const read = vi.fn<AgentForkOriginGateway["read"]>().mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }))
    .mockResolvedValue({ schemaVersion: 1, conversationId: fixture.conversationId, origin: null });
  const onOpen = vi.fn();
  const render = (owner: string) => root.render(<ConversationForkOrigin key={owner} conversationId={fixture.conversationId} binding={{ kind: "note", resourceId: owner }} onOpen={onOpen} gateway={{ read }} />);
  try {
    await act(async () => render("one")); expect(host.textContent).toBe("");
    await act(async () => render("two")); await act(async () => finish({ ...fixture, schemaVersion: 1 }));
    expect(host.textContent).toBe(""); expect(onOpen).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); host.remove(); }
});
it("silently reads lineage while leaving the surrounding input available", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let finish!: (value: Awaited<ReturnType<AgentForkOriginGateway["read"]>>) => void;
  const read = vi.fn<AgentForkOriginGateway["read"]>().mockImplementation(() => new Promise(resolve => {finish = resolve;}));
  const onOpen = vi.fn();
  try {
    await act(async () => root.render(<><ConversationForkOrigin conversationId={fixture.conversationId} binding={{kind:"note", resourceId:"note"}} onOpen={onOpen} gateway={{read}} /><textarea aria-label="对话输入" /></>));
    expect(host.querySelector('[role="status"]')).toBeNull();
    expect(host.textContent).not.toContain("正在读取");
    const input = host.querySelector("textarea")!;
    expect(input.disabled).toBe(false);
    input.focus(); expect(document.activeElement).toBe(input);
    await act(async () => finish({...fixture, schemaVersion: 1}));
    expect(host.textContent).toContain(fixture.origin.parent.title);
    expect(document.activeElement).toBe(input);
    expect(onOpen).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

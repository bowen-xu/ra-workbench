/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { agentDraftInput, insertAgentReference, type AgentReferenceInput } from "@research-assistant/core/agent";
import { AgentUserMessage } from "./AgentUserMessage";
import type { WritingAgentMessage } from "./presentation/WritingAgentMessage";
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
const tool: AgentReferenceInput = { type: "mention", name: "同名资源", target: { kind: "tool", serverId: "server", environmentId: "test", revision: "v1" } };
const file: AgentReferenceInput = { type: "mention", name: "同名资源", target: { kind: "file", sourceId: "paper", resourceId: "main.tex", revision: "v2" } };
const skill: AgentReferenceInput = { type: "skill", name: "写作", identity: { environment_id: "app", source_id: "academic/SKILL.md", revision: "v3" } };
function message(input: AgentReferenceInput): WritingAgentMessage {
  const draft = insertAgentReference({ text: "🧪 用 ", bindings: [] }, input);
  return { id: "turn/user", turnId: "turn", role: "user", content: draft.text, userInput: agentDraftInput(draft) };
}

it("preserves Unicode text, order and exact identities for skills, files and tools", async () => {
  let draft = { text: "🧪 用 ", bindings: [] } as import("@research-assistant/core/agent").AgentInputDraft;
  for (const input of [tool, file, skill]) draft = insertAgentReference(draft, input);
  await act(async () => root.render(<AgentUserMessage message={{ ...message(tool), content: draft.text, userInput: agentDraftInput(draft) }} />));
  expect([...host.querySelectorAll('p button')].map(button => button.getAttribute("aria-label"))).toEqual(["工具 同名资源", "文件 同名资源", "技能 写作"]);
  expect(host.querySelector('p')?.textContent).toBe("🧪 用 同名资源 同名资源 写作 ");
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="文件 同名资源"]')!.click());
  expect(document.querySelector("dialog")).toBeNull();
});

it("leaves typed @names and records without metadata as literal text", async () => {
  await act(async () => root.render(<AgentUserMessage message={{ id: "legacy", role: "user", content: "@同名资源 <tag>" }} />));
  expect(host.textContent).toBe("@同名资源 <tag>");
  expect(host.querySelector('button')).toBeNull();
  expect(host.querySelector('tag')).toBeNull();
});

it("reads frozen metadata without relying on the current catalog, and ignores stale results", async () => {
  const original: WritingAgentMessage = { id: "turn/user", role: "user", content: message(tool).content, rawData: {} };
  let resolve!: (value: unknown) => void;
  const read = vi.fn(() => new Promise<unknown>(done => { resolve = done; }));
  await act(async () => root.render(<AgentUserMessage message={original} read={read} />));
  expect(read).toHaveBeenCalledOnce();
  await act(async () => root.render(<AgentUserMessage message={{ id: "other", role: "user", content: "普通消息" }} read={read} />));
  await act(async () => resolve({ userInput: message(tool).userInput }));
  expect(host.textContent).toBe("普通消息");
  expect(host.querySelector('button')).toBeNull();
  await act(async () => root.render(<AgentUserMessage message={original} read={async () => ({ userInput: message(tool).userInput })} />));
  expect(host.querySelector('[aria-label="工具 同名资源"]')).not.toBeNull();
});

it("rejects mismatched frozen text rather than assigning an identity to another message", async () => {
  const original: WritingAgentMessage = { id: "turn/user", role: "user", content: "另一条消息", rawData: {} };
  await act(async () => root.render(<AgentUserMessage message={original} read={async () => ({ userInput: message(tool).userInput })} />));
  expect(host.textContent).toBe("另一条消息");
  expect(host.querySelector('button')).toBeNull();
});

it("collapses overflowing messages, expands search targets, and allows collapsing them again", async () => {
  const height = vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(400);
  const bounds = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 600, 200));
  const style = vi.spyOn(globalThis, "getComputedStyle").mockReturnValue({lineHeight: "20px"} as CSSStyleDeclaration);
  try {
    const long = {id: "long", role: "user" as const, content: "line\n".repeat(30)};
    await act(async () => root.render(<AgentUserMessage message={long} />));
    const button = host.querySelector<HTMLButtonElement>(".agent-user-message-disclosure button")!;
    expect(button.getAttribute("aria-label")).toBe("展开");
    expect(host.querySelector(".agent-user-message-ellipsis")?.textContent).toBe("...");
    await act(async () => button.click());
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(host.querySelector("p")?.textContent).toBe(long.content);
    await act(async () => button.click());
    expect(button.getAttribute("aria-expanded")).toBe("false");
    await act(async () => root.render(<AgentUserMessage message={long} reveal />));
    expect(button.getAttribute("aria-expanded")).toBe("true");
    await act(async () => button.click());
    expect(button.getAttribute("aria-expanded")).toBe("false");
    bounds.mockReturnValue(new DOMRect()); height.mockReturnValue(0);
    await act(async () => root.render(<AgentUserMessage message={long} />));
    expect(button.isConnected).toBe(true);
    expect(button.getAttribute("aria-expanded")).toBe("false");
  } finally {height.mockRestore(); style.mockRestore(); bounds.mockRestore();}
});

/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { TextPatchReview } from "./TextPatchReview";
import { AgentTextChangeCard } from "./AgentTextChangeCard";
import { selectAppStorage } from "../../runtime/appStorage";
import { OPEN_AGENT_CONVERSATION, type OpenAgentConversation } from "./openAgentConversation";
const mock = vi.hoisted(() => ({ review: vi.fn(), decide: vi.fn(), notifyError: vi.fn(), open: vi.fn(), observers: new Set<() => void>() }));
vi.mock("./sourceNavigation", () => ({ openSourceLink: mock.open }));
vi.mock("../../services/notifications", () => ({ notify: { error: mock.notifyError } }));
vi.mock("./textPatchGateway", () => ({
  textPatchGateway: mock,
  subscribeTextPatches: (listener: () => void) => { mock.observers.add(listener); return () => mock.observers.delete(listener); },
  notifyTextPatchesChanged: () => mock.observers.forEach(listener => listener()),
}));
vi.mock("./AgentSourceLink", () => ({ AgentSourceLink: ({ children }: { children: React.ReactNode }) => <span>{children}</span> }));
vi.mock("./presentation/WritingTextDiff", () => ({ WritingTextDiff: ({ before, after }: { before: string; after: string }) => <pre>{before} → {after}</pre> }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const pending = { proposalId:"proposal", generation:3, state:"pending", baselineMatches:true, pages:[{pageId:"page", path:"source://notes/page", title:"笔记", kind:"update", before:"旧文", after:"新文", expectedRevision:1}] };
beforeEach(() => {
  mock.open.mockReset(); mock.review.mockReset().mockResolvedValue(pending); mock.decide.mockReset(); mock.notifyError.mockReset();
  const values = new Map<string,string>();
  selectAppStorage({ get length(){return values.size;}, key:index => [...values.keys()][index] ?? null, getItem:key => values.get(key) ?? null,
    setItem:(key,value) => {values.set(key,value);}, removeItem:key => {values.delete(key);}, clear:() => values.clear() });
});
async function mount(element: React.ReactNode) {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(element));
  const button = (label:string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === label)!;
  return { host, button, close:async () => { await act(async () => root.unmount()); host.remove(); } };
}
it("shows a whole-group proposal separately from applied files and failed targets", async () => {
  const view = await mount(<AgentTextChangeCard conversationId="conversation" content={JSON.stringify({ targets:[{path:"source://writings/id/a.tex",name:"a.tex",status:"applied",message:"copy"},{path:"source://notes/page",name:"研究提纲",status:"proposed",message:"candidate",proposalId:"proposal"},{path:"source://notes/denied",name:"未授权笔记",status:"failed",message:"outside grant"}] })} />);
  try { await vi.waitFor(() => expect(view.host.querySelector("header strong")?.textContent).toBe("Edited file 笔记"));
    expect(view.host.textContent).toContain("执行副本已修改"); expect(view.host.textContent).toContain("outside grant");
    expect(view.host.textContent).toContain("笔记"); expect(view.host.textContent).not.toContain("source://notes/page");
    expect(view.host.textContent).not.toContain("page.md");
    expect(view.button("接受整组")).toBeUndefined();
    expect(view.button("拒绝整组")).toBeUndefined();
    expect(view.button("继续调整")).toBeUndefined();
    expect(view.button("Undo").closest("header")).not.toBeNull();
  } finally { await view.close(); }
});
it("shows a saved pending note once through the shared Diff card", async () => {
  mock.review.mockResolvedValue({ ...pending, persistenceVersion: 2 });
  const view = await mount(<AgentTextChangeCard conversationId="conversation" content={JSON.stringify({ targets: [
    { path: "source://notes/page", name: "笔记", status: "applied", message: "saved with pending review", proposalId: "proposal" },
  ] })} />);
  try {
    await vi.waitFor(() => expect(view.host.querySelector("header strong")?.textContent).toBe("Edited file 笔记"));
    expect(view.host.textContent).not.toContain("执行副本已修改");
    expect(view.host.querySelector("ul")).toBeNull();
    expect(view.button("Undo")).toBeDefined();
  } finally { await view.close(); }
});
it.each(["pending", "accept"])("chat Undo resolves %s using the existing decision owner", async state => {
  mock.review.mockResolvedValue({ ...pending, state });
  mock.decide.mockImplementation(async request => { mock.review.mockResolvedValue({ ...pending, state: request.action }); return request; });
  const view = await mount(<TextPatchReview conversationId="conversation" proposalId="proposal" presentation="chat" />);
  try {
    await vi.waitFor(() => expect(view.button("Undo")).toBeDefined());
    expect(view.host.querySelector("header strong")?.textContent).toBe("Edited file 笔记");
    await act(async () => view.button("Undo").click());
    expect(mock.decide).toHaveBeenCalledTimes(1);
    expect(mock.decide).toHaveBeenCalledWith(expect.objectContaining({ conversationId: "conversation", proposalId: "proposal", generation: 3,
      action: state === "pending" ? "reject" : "undo" }));
    await vi.waitFor(() => expect(view.button("Undo")).toBeUndefined());
    expect(view.host.querySelector("header strong")?.textContent).toBe("Edited file 笔记");
    expect(view.host.querySelector(".agent-change-summary-footer")).toBeNull();
  } finally { await view.close(); }
});
it.each(["pending", "accept"])("chat Undo preserves newer manual changes for a stale %s proposal", async state => {
  mock.review.mockResolvedValue({ ...pending, state, baselineMatches: false });
  const view = await mount(<TextPatchReview conversationId="conversation" presentation="chat" />);
  try {
    await vi.waitFor(() => expect(view.button("Undo")).toBeDefined());
    expect(view.button("Undo").disabled).toBe(state === "accept");
    if (state === "pending") {
      mock.decide.mockImplementation(async request => request);
      await act(async () => view.button("Undo").click());
      expect(mock.decide).toHaveBeenCalledWith(expect.objectContaining({ action: "reject" }));
    } else expect(mock.decide).not.toHaveBeenCalled();
  } finally { await view.close(); }
});
it.each(["pending", "accept", "undo", "reject"])("keeps the frozen chat summary free of version warnings in %s state", async state => {
  mock.review.mockResolvedValue({ ...pending, state, persistenceVersion: 2, baselineMatches: false, wholeUndoAvailable: false });
  const view = await mount(<TextPatchReview conversationId="conversation" presentation="chat" />);
  try {
    expect(view.host.querySelector("header strong")?.textContent).toBe("Edited file 笔记");
    expect(view.host.querySelector('p[role="status"], [role="alert"]')).toBeNull();
    expect(view.host.textContent).not.toContain("正式版本已变化");
    expect(mock.notifyError).not.toHaveBeenCalled();
    expect(mock.decide).not.toHaveBeenCalled();
    if (state === "pending" || state === "accept") expect(view.button("Undo").disabled).toBe(true);
    else expect(view.button("Undo")).toBeUndefined();
  } finally { await view.close(); }
});
it("reports a rejected chat Undo through the shared notification without changing the historical card", async () => {
  mock.review.mockResolvedValue({ ...pending, state: "accept" });
  mock.decide.mockRejectedValueOnce({ status: "failed", code: "revision_conflict", message: "revision conflict" });
  const view = await mount(<TextPatchReview conversationId="conversation" presentation="chat" />);
  try {
    await act(async () => view.button("Undo").click());
    expect(mock.notifyError).toHaveBeenCalledTimes(1);
    expect(mock.notifyError).toHaveBeenCalledWith("撤销未应用：revision conflict", {
      description: expect.stringContaining(`请求：${mock.decide.mock.calls[0]![0].requestId}`),
    });
    expect(mock.notifyError.mock.calls[0]![1].description).toContain("revision_conflict");
    expect(view.host.querySelector('[role="alert"]')).toBeNull();
    expect(view.host.textContent).not.toContain("revision conflict");
    expect(view.host.querySelector("header strong")?.textContent).toBe("Edited file 笔记");
    expect(view.button("恢复原操作")).toBeUndefined();
  } finally { await view.close(); }
});
it("restores an unknown chat Undo with the original request identity", async () => {
  mock.decide.mockRejectedValueOnce(new Error("lost response"));
  let view = await mount(<TextPatchReview conversationId="conversation" presentation="chat" />);
  await vi.waitFor(() => expect(view.button("Undo")).toBeDefined());
  await act(async () => view.button("Undo").click());
  const original = mock.decide.mock.calls[0]![0];
  expect(mock.notifyError).toHaveBeenCalledTimes(1);
  expect(mock.notifyError.mock.calls[0]![0]).toContain("lost response");
  expect(view.host.querySelector('[role="alert"]')).toBeNull();
  await view.close();
  view = await mount(<TextPatchReview conversationId="conversation" presentation="chat" />);
  try {
    await vi.waitFor(() => expect(view.button("恢复原操作")).toBeDefined());
    expect(mock.notifyError).toHaveBeenCalledTimes(1);
    expect(view.button("Undo").disabled).toBe(true);
    mock.decide.mockImplementation(async request => request);
    await act(async () => view.button("恢复原操作").click());
    expect(mock.decide.mock.calls[1]![0]).toEqual(original);
  } finally { await view.close(); }
});
it("accepts once and restores the original decision identity after an unknown response and remount", async () => {
  mock.decide.mockRejectedValueOnce(new Error("lost response"));
  let view = await mount(<TextPatchReview conversationId="conversation" proposalId="proposal" />);
  await vi.waitFor(() => expect(view.button("接受整组")).toBeDefined());
  await act(async () => view.button("接受整组").click());
  await vi.waitFor(() => expect(view.button("恢复原操作")).toBeDefined());
  const original = mock.decide.mock.calls[0]![0];
  await view.close();
  view = await mount(<TextPatchReview conversationId="conversation" proposalId="proposal" />);
  try {
    await vi.waitFor(() => expect(view.button("恢复原操作")).toBeDefined());
    expect(view.button("接受整组").disabled).toBe(true);
    mock.decide.mockImplementation(async request => { mock.review.mockResolvedValue({ ...pending, state:"accept" }); return request; });
    await act(async () => view.button("恢复原操作").click());
    expect(mock.decide.mock.calls[1]![0]).toEqual(original);
    await vi.waitFor(() => expect(view.host.textContent).toContain("笔记修改 · 已保存"));
    expect(view.button("撤销整组").disabled).toBe(false);
  } finally { await view.close(); }
});
it("blocks acceptance of stale saved content while leaving whole-group rejection available", async () => {
  mock.review.mockResolvedValue({ ...pending, baselineMatches:false });
  const view = await mount(<TextPatchReview conversationId="conversation" pageId="page" />);
  try { await vi.waitFor(() => expect(view.button("接受整组")).toBeDefined());
    expect(view.button("接受整组").disabled).toBe(true); expect(view.button("拒绝整组").disabled).toBe(false);
    expect(view.host.textContent).toContain("旧文 → 新文");
  } finally { await view.close(); }
});
it("releases a definitively failed decision and permits rejection with a new identity", async () => {
  mock.decide.mockRejectedValueOnce({ status: "failed", message: "revision conflict" });
  const view = await mount(<TextPatchReview conversationId="conversation" pageId="page" />);
  try {
    await vi.waitFor(() => expect(view.button("接受整组")).toBeDefined());
    await act(async () => view.button("接受整组").click());
    expect(view.button("恢复原操作")).toBeUndefined();
    expect(view.button("拒绝整组").disabled).toBe(false);
    mock.decide.mockImplementation(async request => { mock.review.mockResolvedValue({ ...pending, state: "reject" }); return request; });
    await act(async () => view.button("拒绝整组").click());
    expect(mock.decide.mock.calls[1]![0].requestId).not.toBe(mock.decide.mock.calls[0]![0].requestId);
    expect(view.host.textContent).toContain("已拒绝");
  } finally { await view.close(); }
});
it("continues in the original conversation without issuing a save or model submission", async () => {
  const opened = vi.fn();
  const navigate = (event: Event) => { event.preventDefault(); const detail = (event as CustomEvent<OpenAgentConversation>).detail; opened(detail.id); detail.resolve(); };
  window.addEventListener(OPEN_AGENT_CONVERSATION, navigate);
  const view = await mount(<TextPatchReview conversationId="original-conversation" pageId="page" />);
  try {
    await vi.waitFor(() => expect(view.button("继续调整")).toBeDefined());
    await act(async () => view.button("继续调整").click());
    expect(opened).toHaveBeenCalledWith("original-conversation"); expect(mock.decide).not.toHaveBeenCalled();
  } finally { window.removeEventListener(OPEN_AGENT_CONVERSATION, navigate); await view.close(); }
});

it("opens edited notes directly from single and multiple entries without comparison content", async () => {
  for (const pages of [pending.pages, [...pending.pages, {...pending.pages[0]!, pageId:"other", title:"第二份笔记"}]]) {
    mock.review.mockResolvedValue({...pending, pages});
    const view = await mount(<TextPatchReview conversationId="conversation" presentation="chat" />);
    try {
      await act(async () => (view.host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-list > button:last-child, .agent-change-summary-file")!).click());
      expect(mock.open).toHaveBeenLastCalledWith('source://notes/' + pages.at(-1)!.pageId);
      expect(view.host.querySelector("dialog, [role=dialog], pre")).toBeNull();
      expect(view.host.textContent).not.toContain("旧文");
    } finally { await view.close(); }
  }
});

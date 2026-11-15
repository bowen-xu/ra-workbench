/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import { AgentTextChangeCard } from "./AgentTextChangeCard";

const mock = vi.hoisted(() => ({ get: vi.fn(), getVersion: vi.fn(), review: vi.fn(), open: vi.fn(), changed: undefined as undefined | ((value: { resourceId: string; paperId: string | null }) => void) }));
vi.mock("../../services/structuredResourceGateway", () => ({ getStructuredResourceGateway: () => ({ ...mock,
  subscribeChanges: async (changed: typeof mock.changed) => { mock.changed = changed; return () => { mock.changed = undefined; }; },
}) }));
vi.mock("./sourceNavigation", () => ({ openSourceLink: mock.open }));
vi.mock("./textPatchGateway", () => ({ notifyTextPatchesChanged: vi.fn() }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const saved: StructuredResource = { id: "portrait", title: "文献画像", kind: "portrait", sourceUri: "source://library/paper/portrait", revision: 1,
  content: { claims: [{ text: "Saved claim" }] }, reviewBaseline: null, pendingFieldIds: ["claim"], reviewedFieldIds: [] };
beforeEach(() => {
  mock.open.mockReset(); mock.get.mockReset().mockResolvedValue(saved); mock.getVersion.mockReset().mockResolvedValue(saved);
  mock.review.mockReset().mockResolvedValue({ ...saved, revision: 2, content: null, pendingFieldIds: [] });
});
async function mount(targets = [{ path: saved.sourceUri, name: saved.title, status: "applied", message: "saved",
  structuredResource: { resourceId: saved.id, revision: 1, receipt: { requestId: "change" } } }]) {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(<AgentTextChangeCard content={JSON.stringify({ targets })} />));
  const button = (label: string): HTMLButtonElement => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === label)!;
  return { host, button, close: async () => { await act(async () => root.unmount()); host.remove(); } };
}
it("shows native structured receipts through the shared Edited card and its original review owner", async () => {
  const view = await mount();
  try {
    expect(view.host.querySelector("header strong")?.textContent).toBe("Edited file 文献画像");
    expect(view.host.querySelector("ul")).toBeNull();
    expect(view.host.textContent).not.toContain("执行副本");
    expect(view.button("Undo").disabled).toBe(false);
    expect(mock.getVersion).toHaveBeenCalledExactlyOnceWith("portrait", 1);
    await act(async () => view.button("Undo").click());
    expect(mock.review).toHaveBeenCalledWith(expect.objectContaining({ resourceId: "portrait", expectedRevision: 1, action: "undo", scope: "all" }));
    expect(view.button("Undo").disabled).toBe(true);
  } finally { await view.close(); }
});
it("retains frozen Diff after live changes and refuses to erase later edits", async () => {
  const view = await mount();
  try {
    await act(async () => view.button(saved.title).click());
    expect(mock.open).toHaveBeenLastCalledWith(saved.sourceUri);
    expect(document.querySelector("dialog")).toBeNull();
    mock.get.mockResolvedValue({ ...saved, revision: 2, content: { claims: [{ text: "Later manual edit" }] } });
    await act(async () => mock.changed?.({ resourceId: saved.id, paperId: null }));
    expect(view.button("Undo").disabled).toBe(true);
    expect(document.querySelector("[data-frozen-diff]")).toBeNull();
    expect(mock.getVersion).toHaveBeenCalledTimes(1);
  } finally { await view.close(); }
});
it("does not undo earlier pending work when the original baseline differs", async () => {
  mock.getVersion.mockImplementation(async (_id: string, revision: number) => ({ ...saved, revision, content: revision === 1 ? { oldPending: true } : saved.content }));
  mock.get.mockResolvedValue({ ...saved, revision: 2 });
  const view = await mount([{ path: saved.sourceUri, name: saved.title, status: "applied", message: "saved",
    structuredResource: { resourceId: saved.id, revision: 2, receipt: { requestId: "later-change" } } }]);
  try { expect(view.button("Undo").disabled).toBe(true); expect(mock.review).not.toHaveBeenCalled(); }
  finally { await view.close(); }
});
it("keeps the card and reports unavailable snapshots instead of inventing a Diff", async () => {
  mock.getVersion.mockRejectedValue(new Error("not_found: missing snapshot"));
  const view = await mount();
  try {
    expect(view.host.querySelector("header strong")?.textContent).toBe("Edited file 文献画像");
    expect(view.host.querySelector('[role="alert"]')?.textContent).toContain("not_found");
    expect(view.button("Undo").disabled).toBe(true);
    expect(view.button(saved.title).disabled).toBe(false);
  } finally { await view.close(); }
});
it("rechecks the live target before Undo and preserves concurrent edits", async () => {
  const view = await mount();
  try {
    mock.get.mockResolvedValue({ ...saved, revision: 2, content: { manual: "preserve" } });
    await act(async () => view.button("Undo").click());
    expect(mock.review).not.toHaveBeenCalled();
    expect(view.host.querySelector('[role="alert"]')?.textContent).toContain("资源已变化");
  } finally { await view.close(); }
});
it("recovers an uncertain Undo using the same request identity", async () => {
  mock.review.mockRejectedValueOnce(new Error("connection lost"));
  const view = await mount();
  try {
    await act(async () => view.button("Undo").click());
    const original = mock.review.mock.calls[0]![0];
    // The original write might have succeeded. Recovery must not depend on
    // the old revision still being current or mint a second request.
    mock.get.mockResolvedValue({ ...saved, revision: 2, content: null, pendingFieldIds: [] });
    await act(async () => view.button("恢复 Undo").click());
    expect(mock.review.mock.calls[1]![0]).toEqual(original);
    expect(view.button("Undo").disabled).toBe(true);
  } finally { await view.close(); }
});

function twoResources() {
  const template: StructuredResource = { ...saved, id: "template", kind: "readingTemplate", title: "本文阅读模板",
    sourceUri: "source://agents/skills/edit-structured-resources/assets/reading/template.json", content: { fields: [{ title: "Question" }] } };
  const resources = [saved, template];
  mock.get.mockImplementation(async (id: string) => resources.find(resource => resource.id === id)!);
  mock.getVersion.mockImplementation(async (id: string) => resources.find(resource => resource.id === id)!);
  mock.review.mockImplementation(async (request: { resourceId: string }) => ({
    ...resources.find(resource => resource.id === request.resourceId)!, revision: 2, content: null, pendingFieldIds: [],
  }));
  return resources.map(resource => ({ path: resource.sourceUri, name: resource.title, status: "applied", message: "saved",
    structuredResource: { resourceId: resource.id, revision: 1, receipt: { requestId: `change-${resource.id}` } } }));
}
it("groups the turn's resources into one card with file rows, combined statistics and one Undo", async () => {
  const view = await mount(twoResources());
  try {
    expect(view.host.querySelectorAll('.writing-agent-inline-diff-card')).toHaveLength(1);
    expect(view.host.querySelector('header strong')?.textContent).toBe("Edited 2 files");
    const rows = view.host.querySelectorAll('.writing-agent-inline-diff-list > button');
    expect(rows).toHaveLength(2);
    expect(rows[0]?.textContent).toContain("文献画像"); expect(rows[1]?.textContent).toContain("本文阅读模板");
    const additions = [...rows].reduce((sum, row) => sum + Number(row.querySelector('b')!.textContent!.slice(1)), 0);
    expect(view.host.querySelector('.writing-agent-inline-diff-counts b')?.textContent).toBe(`+${additions}`);
    expect(view.host.querySelectorAll('.writing-agent-inline-diff-undo')).toHaveLength(1);
    await act(async () => (rows[1] as HTMLButtonElement).click());
    expect(mock.open).toHaveBeenLastCalledWith("source://agents/skills/edit-structured-resources/assets/reading/template.json");
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    await act(async () => view.button("Undo").click());
    expect(mock.review.mock.calls.map(call => call[0].resourceId)).toEqual(["portrait", "template"]);
    expect(view.button("Undo").disabled).toBe(true);
  } finally { await view.close(); }
});
it("checks every resource before group Undo starts writing", async () => {
  const targets = twoResources();
  const view = await mount(targets);
  try {
    const originalGet = mock.get.getMockImplementation()!;
    mock.get.mockImplementation(async (id: string) => {
      const value = await originalGet(id);
      return id === "template" ? { ...value, revision: 2 } : value;
    });
    await act(async () => view.button("Undo").click());
    expect(mock.review).not.toHaveBeenCalled();
    expect(view.host.querySelector('[role="alert"]')?.textContent).toContain("本文阅读模板：资源已变化");
  } finally { await view.close(); }
});
it("reports partial Undo and recovers only the uncertain resource with its original identity", async () => {
  const targets = twoResources();
  const originalReview = mock.review.getMockImplementation()!;
  mock.review.mockImplementation(async request => {
    if (request.resourceId === "template") throw new Error("connection lost");
    return originalReview(request);
  });
  const view = await mount(targets);
  try {
    await act(async () => view.button("Undo").click());
    expect(view.host.textContent).toContain("已撤销 1/2 项修改");
    const originalRequest = mock.review.mock.calls[1]![0];
    mock.review.mockImplementation(originalReview);
    await act(async () => view.button("恢复 Undo").click());
    expect(mock.review.mock.calls.map(call => call[0].resourceId)).toEqual(["portrait", "template", "template"]);
    expect(mock.review.mock.calls[2]![0]).toEqual(originalRequest);
    expect(view.host.textContent).not.toContain("已撤销 1/2 项修改");
    expect(view.button("Undo").disabled).toBe(true);
  } finally { await view.close(); }
});

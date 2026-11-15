// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {registerNoteReviewSavePort} from "../editor/noteTextEditorPorts";
import {NoteSaveState} from "./NoteSaveState";

it("shows the saved icon without a pending-review badge when review changes remain", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const unregister = registerNoteReviewSavePort("review-badge-test", {
    flush: async () => {}, retry: async () => {},
    getState: () => ({status: "saved", pendingChanges: 3}),
  });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<NoteSaveState status="saved" message={undefined} pageId="review-badge-test"/>));
    expect(host.textContent).toBe("");
    expect(host.querySelector('[role="status"][aria-label="已保存"] svg')).not.toBeNull();
    expect(host.querySelector(".note-review-state")).toBeNull();
  } finally {
    await act(async () => root.unmount()); host.remove(); unregister(); vi.unstubAllGlobals();
  }
});

it.each([
  ["conflict", "MODEL_UNRESOLVED：输入已保全，尚有内容冲突。", true],
  ["failed", "MODEL_REQUEST_UNSETTLED：原请求等待核对。", true],
  ["failed", "MODEL_LEGACY_BASE_MISSING：旧恢复材料无法重建。", true],
] as const)("distinguishes recoverable content conflict from exceptional state %s / %s",async(status,message,hasMenu)=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  try {
    await act(async()=>root.render(<NoteSaveState status={status} message={message} pageId="status-classification"/>));
    const state=host.querySelector<HTMLElement>('[role="status"]')!;
    expect(state.getAttribute("aria-label")).toContain(message);
    expect(state.getAttribute("aria-label")?.includes("请在正文内使用 Undo／Keep、Take／Both 处理")).toBe(status==="conflict");
    expect(state.querySelector('svg.lucide-circle-alert')).not.toBeNull();
    const trigger=state.querySelector<HTMLButtonElement>('button');
    expect(!!trigger).toBe(hasMenu);
    await act(async()=>{(trigger??state).click();});
    expect(!!document.querySelector('[role="menu"][aria-label="笔记恢复与诊断"]')).toBe(hasMenu);
  }finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});

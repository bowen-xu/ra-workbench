// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { PaperPortraitPanel } from "../../../../../../../plugins/ra-literature-library/src/features/ai/PaperPortraitPanel";
import { panelItem } from "../../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";
import { selectAppStorage } from "../../../../../../apps/web/src/core/runtime/appStorage";

vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({notify: {error: vi.fn(), success: vi.fn()}}));
it("handles panel history from the outer header and closes only on an outside click", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);selectAppStorage(localStorage);
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const item={...panelItem(),id:"portrait-interaction"};
  const gateway={listCandidates:vi.fn(async()=>[])} as unknown as AiAssistanceGateway;
  const button=(name:string)=>document.querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`)!;
  try {
    await act(async()=>root.render(<><button>Outside</button><PaperPortraitPanel item={item} gateway={gateway}/></>));
    await act(async()=>button("查看画像详情").click());
    const dialog=document.querySelector('[role="dialog"][aria-label="文献画像详情"]')!;
    await act(async()=>button(`移除来源 ${item.attachments[0]!.displayName}`).click());
    expect(dialog.textContent).toContain("未添加来源");
    const key=async(key:string,shiftKey=false)=>act(async()=>button("关闭画像详情").dispatchEvent(new KeyboardEvent("keydown",{key,ctrlKey:true,shiftKey,bubbles:true,cancelable:true})));
    await key("z");expect(dialog.textContent).not.toContain("未添加来源");
    await key("z",true);expect(dialog.textContent).toContain("未添加来源");
    await key("y");expect(dialog.textContent).toContain("未添加来源");
    await act(async()=>host.querySelector("button")!.focus());
    expect(document.body.contains(dialog)).toBe(true);
    await act(async()=>{
      const outside=host.querySelector("button")!;
      outside.dispatchEvent(new MouseEvent("mousedown",{bubbles:true}));
      outside.dispatchEvent(new MouseEvent("mouseup",{bubbles:true}));outside.click();
    });
    await vi.waitFor(()=>expect(document.body.contains(dialog)).toBe(false));
  } finally {await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});

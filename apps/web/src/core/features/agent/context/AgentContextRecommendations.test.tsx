// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import type {AgentContextSelection} from "@research-assistant/core/agent";
import {AgentContextRecommendations} from "./AgentContextRecommendations";
import {notify} from "../../../services/notifications";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
it("keeps the primary toggle available without opening an empty list and closes when the last other candidate disappears", async () => {
  const primary: AgentContextSelection = {label: "当前笔记", resource: {kind: "note", resourceId: "current"}};
  const other: AgentContextSelection = {label: "其它笔记", resource: {kind: "note", resourceId: "other"}};
  const onChange = vi.fn(async () => {});
  const errorNotice = vi.spyOn(notify, "error").mockReturnValue("notice");
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const render = (suggestions: AgentContextSelection[], error?: string) => act(async () => root.render(<AgentContextRecommendations primary={primary} selected={[]} suggestions={suggestions} disabled={false} onChange={onChange} {...(error ? {error} : {})}/>));
  try {
    await render([]);
    const disclosure = () => host.querySelector<HTMLButtonElement>('[aria-label="推荐上下文 当前笔记"]')!;
    expect(disclosure().disabled).toBe(true);
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="添加推荐上下文 当前笔记"]')!.click());
    expect(onChange).toHaveBeenCalledWith(primary, false);
    expect(document.querySelector('[role="dialog"][aria-label="推荐上下文"]')).toBeNull();
    await render([other]); expect(disclosure().disabled).toBe(false);
    await act(async () => disclosure().click());
    expect(document.querySelector('[role="dialog"][aria-label="推荐上下文"]')).not.toBeNull();
    await render([]); expect(document.querySelector('[role="dialog"][aria-label="推荐上下文"]')).toBeNull();
    expect(document.body.textContent).not.toContain("暂无其他推荐");
    await render([], "推荐选择保存失败"); expect(errorNotice).toHaveBeenCalledWith("推荐选择保存失败");
    expect(document.querySelector('[role="dialog"][aria-label="推荐上下文"]')).toBeNull();
  } finally {await act(async () => root.unmount()); host.remove(); errorNotice.mockRestore();}
});

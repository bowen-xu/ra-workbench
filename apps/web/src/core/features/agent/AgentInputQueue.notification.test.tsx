/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { notify, type NotificationOptions } from "../../services/notifications";
import { AgentInputQueue } from "./AgentInputQueue";
import { copyText } from "./copyText";

vi.mock("../../services/notifications", () => ({notify: {error: vi.fn(), success: vi.fn()}}));
vi.mock("./copyText", () => ({copyText: vi.fn().mockResolvedValue(true)}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;

it("uses the shared icon action and retains the reload promise", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let finish!: () => void;
  const reload = vi.fn(() => new Promise<void>(resolve => {finish = resolve;}));
  try {
    await act(async () => root.render(<AgentInputQueue
      state={{queue: null, loading: false, editing: false, uncertain: false, error: "读取队列失败，请重试。",
        readFailure: {cause: {code: "closed"}, scope: {conversationId: "11111111-1111-4111-8111-111111111111"}}}}
      disabled={false} onEdit={async () => true} onReload={reload} onRetry={() => undefined} />));
    const options = vi.mocked(notify.error).mock.calls[0]![1] as NotificationOptions;
    expect(vi.mocked(notify.error).mock.calls[0]![0]).toBe("读取队列失败，请重试。");
    expect(options.action).toBeUndefined();
    expect(options.actions).toHaveLength(2);
    const copy = options.actions!.find(item => item.id === "copy")!;
    expect(copy).toMatchObject({label: "复制错误信息"}); expect(copy.icon).toBeTruthy();
    await copy.onClick();
    expect(copyText).toHaveBeenCalledWith(expect.stringContaining("code=closed"));
    expect(copyText).toHaveBeenCalledWith(expect.stringContaining("C=11111111-1111-4111-8111-111111111111"));
    expect(notify.success).toHaveBeenCalledWith("已复制错误信息");
    vi.mocked(copyText).mockResolvedValueOnce(false); await copy.onClick();
    expect(notify.error).toHaveBeenCalledWith("复制错误信息失败，请重试。");
    const action = options.actions!.find(item => item.id === "reload")!;
    expect(action).toMatchObject({id: "reload", label: "重试"});
    expect(action.icon).toBeTruthy();
    const pending = action.onClick(); expect(reload).toHaveBeenCalledOnce();
    expect(pending).toBeInstanceOf(Promise);
    finish(); await pending;
  } finally {await act(async () => root.unmount()); host.remove(); vi.clearAllMocks();}
});

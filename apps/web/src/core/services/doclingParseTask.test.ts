import { afterEach, expect, it, vi } from "vitest";
import type { DoclingGateway, DoclingParseTaskStatus } from "./doclingGateway";
import { doclingTaskMessage, runDoclingParse } from "./doclingParseTask";
import { notify } from "./notifications";
vi.mock("./notifications", () => ({ notify: { loading: vi.fn(() => "toast"), update: vi.fn(), success: vi.fn(), info: vi.fn(), error: vi.fn() } }));
afterEach(() => { vi.useRealTimers(); vi.clearAllMocks(); });
const document = { paperTitle: "合成文献标题", fileName: "fixture.pdf" };
const caption = expect.objectContaining({ props: document });
const task: DoclingParseTaskStatus = { id: "task", state: "running", stage: "connecting", completedPages: 0, totalPages: 0 };
it("shows native phases and actual page counts on the same message", async () => {
  vi.useFakeTimers();
  const detail = { id: "run" };
  const gateway = { startParseTask: vi.fn(async () => task), getParseTask: vi.fn()
    .mockResolvedValueOnce({ ...task, stage: "parsing", completedPages: 2, totalPages: 8 })
    .mockResolvedValueOnce({ ...task, state: "succeeded", parseRunId: "run" }),
    getParseRun: vi.fn(async () => detail) } as unknown as DoclingGateway;
  const parse = runDoclingParse(gateway, "paper", "pdf", document);
  await vi.advanceTimersByTimeAsync(301);
  expect(notify.update).toHaveBeenCalledWith("toast", expect.objectContaining({ message: "正在解析 PDF · 2/8 页" }));
  await vi.advanceTimersByTimeAsync(301);
  expect(await parse).toBe(detail);
  expect(notify.success).toHaveBeenCalledWith("PDF 解析完成", { id: "toast", description: caption });
  expect(gateway.getParseRun).toHaveBeenCalledWith("run");
});
it("keeps a cancelling message until the backend acknowledges termination", async () => {
  vi.useFakeTimers();
  const gateway = { startParseTask: vi.fn(async () => task), cancelParseTask: vi.fn(async () => ({ ...task, state: "cancelling" })),
    getParseTask: vi.fn().mockResolvedValueOnce({ ...task, state: "cancelling" }).mockResolvedValueOnce({ ...task, state: "cancelled" }) } as unknown as DoclingGateway;
  const parse = runDoclingParse(gateway, "paper", "pdf", document);
  await vi.advanceTimersByTimeAsync(1);
  const action = vi.mocked(notify.update).mock.calls.at(-1)![1].actions![0]!;
  await action.onClick();
  expect(action).toMatchObject({ label: "终止解析", icon: expect.anything() });
  expect(gateway.cancelParseTask).toHaveBeenCalledWith("task");
  await vi.advanceTimersByTimeAsync(300);
  expect(notify.info).not.toHaveBeenCalled();
  expect(notify.update).toHaveBeenLastCalledWith("toast", expect.objectContaining({ message: expect.stringContaining("正在终止"), actions: [expect.objectContaining({ disabled: true })] }));
  await vi.advanceTimersByTimeAsync(300);
  expect(await parse).toBeUndefined();
  expect(notify.info).toHaveBeenCalledWith("PDF 解析已终止", { id: "toast", description: caption });
});
it("retains actual native errors and does not invent progress percentages", async () => {
  expect(doclingTaskMessage(task)).toBe("正在连接 Docling 服务…");
  expect(doclingTaskMessage({ ...task, stage: "parsing" })).toBe("正在解析 PDF…");
  const error = { code: "models_unavailable", message: "请下载模型" };
  const gateway = { startParseTask: vi.fn(async () => ({ ...task, state: "failed", error })) } as unknown as DoclingGateway;
  await expect(runDoclingParse(gateway, "paper", "pdf", document)).rejects.toBe(error);
  expect(notify.error).toHaveBeenCalledWith("[models_unavailable] 请下载模型", { id: "toast", description: caption });
});
it("shows response inactivity duration, task identity and cause in the failure notification", async () => {
  const error = { code: "response_timeout", message: "Docling 解析连续 10000 毫秒未收到有效任务状态响应（任务 ID：conversion-123）；最后状态：Docling status HTTP 503" };
  const gateway = { startParseTask: vi.fn(async () => ({ ...task, state: "failed", error })) } as unknown as DoclingGateway;
  await expect(runDoclingParse(gateway, "paper", "pdf", document)).rejects.toBe(error);
  expect(notify.error).toHaveBeenCalledWith(`[response_timeout] ${error.message}`, { id: "toast", description: caption });
});

import { expect, it, vi } from "vitest";
import { getDoclingGateway } from "./doclingGateway";

const native = vi.hoisted(() => ({ invoke: vi.fn(), metadataAvailable: true }));
vi.mock("../runtime/platformCore", () => ({ invoke: native.invoke, isDesktopRuntime: () => true }));
vi.mock("../runtime/appRuntime", () => ({ resolveAppGateway: (name: string, gateway: unknown) =>
  name === "doclingAttachmentMetadata" && native.metadataAvailable
    ? {describe: async () => ({paperTitle: "合成文献", fileName: "fixture.pdf"})} : gateway }));

it("normalizes native failures for parsing, model operations and service connection", async () => {
  const reason = { code: "start_failed", message: "Docling 服务启动超时" };
  native.invoke.mockRejectedValue(reason);
  const gateway = getDoclingGateway();
  for (const call of [() => gateway.parseAttachment("paper", "pdf", { force: true }), () => gateway.connectService(), () => gateway.downloadModels()]) {
    await expect(call()).rejects.toMatchObject({ message: "[start_failed] Docling 服务启动超时", cause: reason });
  }
  const detail = { id: "parse" };
  native.invoke.mockImplementation(async (command: string) => command === "start_docling_parse_task" ? { id: "task", state: "succeeded", parseRunId: "parse" } : detail);
  expect(await gateway.parseAttachment("paper", "pdf", { force: true })).toBe(detail);
  expect(native.invoke).toHaveBeenCalledWith("start_docling_parse_task", { itemId: "paper", attachmentId: "pdf", force: true });
  expect(native.invoke).toHaveBeenLastCalledWith("get_docling_parse_run", { runId: "parse" });
});

it("reports an unavailable source provider before dispatching parser work", async () => {
  native.metadataAvailable = false;
  native.invoke.mockClear();
  try {
    await expect(getDoclingGateway().parseAttachment("paper", "pdf")).rejects.toMatchObject({code: "DOCLING_SOURCE_PROVIDER_UNAVAILABLE"});
    expect(native.invoke).not.toHaveBeenCalled();
  } finally { native.metadataAvailable = true; }
});

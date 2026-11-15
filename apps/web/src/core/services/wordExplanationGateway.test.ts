import { afterEach, describe, expect, it, vi } from "vitest";
import { getWordExplanationGateway } from "./wordExplanationGateway";

const platform = vi.hoisted(() => ({ invoke: vi.fn(), isDesktopRuntime: vi.fn(() => true) }));
vi.mock("../runtime/platformCore", () => platform);
vi.mock("../runtime/appRuntime", () => ({ resolveAppGateway: (_name: string, production: unknown) => production }));
afterEach(() => { vi.clearAllMocks(); platform.isDesktopRuntime.mockReturnValue(true); });

describe("contextual dictionary generation port", () => {
  const request = { word: "work", before: "In this ", after: ", we study vision." };
  it("uses the narrow IPC and cancels without sending a chat message", async () => {
    let finish!: (value: unknown) => void;
    platform.invoke.mockImplementation((command: string) => command === "explain_dictionary_word"
      ? new Promise(resolve => { finish = resolve; }) : Promise.resolve());
    const controller = new AbortController();
    const result = getWordExplanationGateway().explain(request, controller.signal);
    const requestId: unknown = platform.invoke.mock.calls[0]?.[1]?.requestId;
    expect(platform.invoke).toHaveBeenCalledWith("explain_dictionary_word", { requestId, request });
    controller.abort(); expect(platform.invoke).toHaveBeenCalledWith("cancel_word_explanation", { requestId });
    finish({ translation: "Late", explanation: "Late", providerId: "test", modelId: "test" });
    await expect(result).rejects.toMatchObject({ name: "AbortError" });
  });
  it("validates output and propagates safe native errors", async () => {
    platform.invoke.mockResolvedValueOnce({ translation: "本研究", explanation: "这里指本文的研究工作。", providerId: "test", modelId: "test-v1" });
    await expect(getWordExplanationGateway().explain(request)).resolves.toMatchObject({ modelId: "test-v1" });
    platform.invoke.mockResolvedValueOnce({ text: "" });
    await expect(getWordExplanationGateway().explain(request)).rejects.toThrow("响应无效");
    platform.invoke.mockRejectedValueOnce({ code: "invalid_input", message: "请先配置阅读模型" });
    await expect(getWordExplanationGateway().explain(request)).rejects.toThrow("请先配置阅读模型");
  });
  it("does not invoke native generation in an ordinary browser", async () => {
    platform.isDesktopRuntime.mockReturnValue(false);
    await expect(getWordExplanationGateway().explain(request)).rejects.toThrow("桌面应用");
    expect(platform.invoke).not.toHaveBeenCalled();
  });
});

// @vitest-environment happy-dom
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {parseManuscriptPath} from "@research-assistant/writing";
import {selectAppRuntime, type AppRuntime} from "../../../core/runtime/appRuntime";
import {resolveWritingRuntimeGateway, getWritingNoteAssetNativeGateway} from "../../../core/public/writingRuntime";
import {getWritingGateway} from "@research-assistant/writing-projects-plugin/writingGateway";
import {getWritingCheckpointGateway} from "@research-assistant/writing-projects-plugin/writingCheckpointGateway";
import {configureBuiltinServices} from "../configureBuiltinServices";
import {tauriWritingGateway} from "./tauriWritingGateway";

const fixture = vi.hoisted(() => ({invoke: vi.fn(), getLiterature: vi.fn(() => ({}))}));
vi.mock("@tauri-apps/api/core", () => ({invoke: fixture.invoke, isTauri: () => false}));
vi.mock("@research-assistant/literature-library-plugin/literatureGateway", () => ({getLiteratureGateway: fixture.getLiterature}));
vi.mock("@research-assistant/literature-library-plugin/paperPreparation", () => ({createPreparedPaperGenerationGateway: () => ({})}));
vi.mock("../../../core/services/aiAssistanceGateway", () => ({getAiAssistanceTransport: () => ({})}));
vi.mock("../../../core/services/doclingGateway", () => ({getDoclingGateway: () => ({})}));

function runtime(mode: AppRuntime["mode"]): AppRuntime {
  return {mode, storage: window.localStorage, gateways: {}, dispose() {}};
}
beforeEach(() => {vi.clearAllMocks(); selectAppRuntime(runtime("browser"));});
afterEach(() => {vi.restoreAllMocks(); selectAppRuntime(runtime("browser"));});

it("binds the actual desktop adapter before consuming it and retains one observed owner", async () => {
  const selected = runtime("desktop");
  selectAppRuntime(selected);
  configureBuiltinServices(selected);
  const gateway = getWritingGateway();
  expect(gateway).toBe(selected.gateways.getWritingGateway);
  expect(gateway).not.toBe(tauriWritingGateway);
  expect(gateway.supportsCompilation).toBe(true);
  expect(getWritingCheckpointGateway()).toBe(selected.gateways.getWritingCheckpointGateway);
  expect(getWritingNoteAssetNativeGateway()).toBe(selected.gateways.getWritingNoteAssetGateway);
  configureBuiltinServices(selected);
  expect(getWritingGateway()).toBe(gateway);
  const changed = vi.fn(); window.addEventListener("research-assistant:writing-notes-changed", changed);
  try {
    const request = {manuscriptId: "manuscript", relativePath: parseManuscriptPath("main.tex"), content: "source", expectedSha256: "before"};
    const receipt = {relativePath: "main.tex", content: "source", sha256: "after"};
    fixture.invoke.mockResolvedValueOnce(receipt);
    await expect(gateway.saveFile(request)).resolves.toBe(receipt);
    expect(fixture.invoke).toHaveBeenCalledExactlyOnceWith("save_manuscript_file", {request});
    expect(changed).toHaveBeenCalledTimes(1);
    const failure = {code: "revision_conflict", requestId: "request-7", message: "preserved"};
    fixture.invoke.mockRejectedValueOnce(failure);
    await expect(getWritingGateway().saveFile(request)).rejects.toBe(failure);
    expect(changed).toHaveBeenCalledTimes(1);
  } finally {window.removeEventListener("research-assistant:writing-notes-changed", changed);}
});

it("keeps the real browser business fallback during construction and normal assembly", async () => {
  const selected = runtime("browser"); selectAppRuntime(selected);
  const before = getWritingGateway();
  expect(before.supportsCompilation).toBe(false);
  configureBuiltinServices(selected);
  expect(selected.gateways.getWritingGateway).toBe(before);
  expect(getWritingGateway()).toBe(before);
  expect(fixture.invoke).not.toHaveBeenCalled();
});

it("preserves an existing override and full-mock selection without replacing its authority", () => {
  const fallback = getWritingGateway();
  const override = {...fallback, supportsCompilation: true};
  const selected = runtime("preview"); selected.gateways.getWritingGateway = override;
  selectAppRuntime(selected);
  configureBuiltinServices(selected);
  expect(selected.gateways.getWritingGateway).toBe(override);
  expect(resolveWritingRuntimeGateway(fallback)).toBe(override);
  expect(getWritingGateway()).toBe(getWritingGateway());
  expect(getWritingGateway().supportsCompilation).toBe(true);
  expect(fixture.getLiterature).not.toHaveBeenCalled();
  expect(fixture.invoke).not.toHaveBeenCalled();
});

it("wires native Agent preview through its existing transport and observed identity", async () => {
  const invoke = vi.fn(async () => []);
  const selected = {...runtime("preview"), nativeAgentPreview: {} as NonNullable<AppRuntime["nativeAgentPreview"]>, invoke};
  selectAppRuntime(selected);
  configureBuiltinServices(selected);
  const gateway = getWritingGateway();
  expect(gateway).toBe(selected.gateways.getWritingGateway);
  expect(gateway.supportsCompilation).toBe(true);
  configureBuiltinServices(selected);
  expect(getWritingGateway()).toBe(gateway);
  await expect(gateway.listFiles("manuscript")).resolves.toEqual([]);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("list_manuscript_files", {manuscriptId: "manuscript"});
  expect(fixture.invoke).not.toHaveBeenCalled();
  expect(fixture.getLiterature).not.toHaveBeenCalled();
});

it("fails closed for missing preview or native assembly without substituting browser execution", () => {
  const fallback = getWritingGateway();
  selectAppRuntime(runtime("preview"));
  expect(() => resolveWritingRuntimeGateway(fallback)).toThrow("预览未覆盖业务网关：getWritingGateway");
  for (const selected of [runtime("desktop"), {...runtime("preview"), nativeAgentPreview: {} as NonNullable<AppRuntime["nativeAgentPreview"]>}]) {
    selectAppRuntime(selected);
    expect(() => getWritingGateway()).toThrow(expect.objectContaining({code: "writing_gateway_unavailable"}));
  }
  expect(fixture.invoke).not.toHaveBeenCalled();
});

it("separates checkpoint native-preview support from desktop-only asset access", () => {
  const checkpoint = getWritingCheckpointGateway();
  const mock = runtime("preview"); mock.gateways.getWritingCheckpointGateway = checkpoint;
  selectAppRuntime(mock); configureBuiltinServices(mock);
  expect(getWritingCheckpointGateway()).toBe(checkpoint);
  expect(() => getWritingNoteAssetNativeGateway()).toThrow(expect.objectContaining({code: "writing_note_asset_gateway_unavailable"}));
  const native = {...runtime("preview"), nativeAgentPreview: {} as NonNullable<AppRuntime["nativeAgentPreview"]>, invoke: vi.fn()};
  selectAppRuntime(native); configureBuiltinServices(native);
  expect(getWritingCheckpointGateway()).toBe(native.gateways.getWritingCheckpointGateway);
  expect(getWritingCheckpointGateway()).not.toBe(checkpoint);
  expect(native.gateways.getWritingNoteAssetGateway).toBeUndefined();
  expect(() => getWritingNoteAssetNativeGateway()).toThrow(expect.objectContaining({code: "writing_note_asset_gateway_unavailable"}));
  expect(native.invoke).not.toHaveBeenCalled();
});

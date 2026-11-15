// @vitest-environment happy-dom
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {getAppRuntime, selectAppRuntime, type AppRuntime} from "../../../core/runtime/appRuntime";
import {resolveResearchExportRuntimeGateway} from "../../../core/public/literatureRuntime";
import type {ResearchExportGateway} from "@research-assistant/literature-library-plugin/contracts/researchExportGateway";
import {configureResearchExport, tauriResearchExportGateway} from "./tauriResearchExportGateway";

const fixture = vi.hoisted(() => ({native: vi.fn()}));
vi.mock("@tauri-apps/api/core", () => ({invoke: fixture.native, isTauri: () => false}));
const previous = getAppRuntime();
const report = {folderName: "synthetic-export", fileCount: 2, objectCount: 3, sourceCount: 1};
const browser: ResearchExportGateway = {exportAll: async () => {throw new Error("browser refusal");}, cancel: async () => false};
const runtime = (mode: AppRuntime["mode"]): AppRuntime => ({mode, storage: window.localStorage, gateways: {}, dispose() {}});
beforeEach(() => {fixture.native.mockReset(); selectAppRuntime(runtime("browser"));});
afterEach(() => selectAppRuntime(previous));

it("binds only the fixed desktop commands and retains the selected identity", async () => {
  const owner = runtime("desktop"); selectAppRuntime(owner); configureResearchExport(owner);
  expect(resolveResearchExportRuntimeGateway(browser)).toBe(tauriResearchExportGateway);
  configureResearchExport(owner);
  expect(owner.gateways.getResearchExportGateway).toBe(tauriResearchExportGateway);
  fixture.native.mockResolvedValueOnce(report).mockResolvedValueOnce(false);
  const gateway = resolveResearchExportRuntimeGateway(browser);
  await expect(gateway.exportAll("/synthetic/destination")).resolves.toEqual(report);
  await expect(gateway.cancel()).resolves.toBe(false);
  expect(fixture.native.mock.calls).toEqual([
    ["export_research_knowledge", {request: {targetDirectory: "/synthetic/destination"}}], ["cancel_research_export"],
  ]);
});

it.each([false, true])("retains the selected preview transport without native fallback (native Agent=%s)", async native => {
  const invoke = vi.fn(async (command: string) => command === "export_research_knowledge" ? report : true);
  const owner: AppRuntime = {...runtime("preview"), invoke,
    ...(native ? {nativeAgentPreview: {} as NonNullable<AppRuntime["nativeAgentPreview"]>} : {})};
  selectAppRuntime(owner); configureResearchExport(owner);
  const gateway = resolveResearchExportRuntimeGateway(browser);
  await expect(gateway.exportAll("/synthetic/preview")).resolves.toEqual(report);
  await expect(gateway.cancel()).resolves.toBe(true);
  expect(invoke.mock.calls).toEqual([
    ["export_research_knowledge", {request: {targetDirectory: "/synthetic/preview"}}], ["cancel_research_export", undefined],
  ]);
  expect(fixture.native).not.toHaveBeenCalled();
});

it("preserves an explicit authority and keeps browser-only refusal", () => {
  const owner = runtime("preview"), override: ResearchExportGateway = {exportAll: vi.fn(), cancel: vi.fn()};
  owner.gateways.getResearchExportGateway = override;
  selectAppRuntime(owner); configureResearchExport(owner);
  expect(resolveResearchExportRuntimeGateway(browser)).toBe(override);
  const ordinary = runtime("browser"); selectAppRuntime(ordinary); configureResearchExport(ordinary);
  expect(ordinary.gateways.getResearchExportGateway).toBeUndefined();
  expect(resolveResearchExportRuntimeGateway(browser)).toBe(browser);
  expect(fixture.native).not.toHaveBeenCalled();
});

it("rejects missing assembly or missing preview transport without invoking native", async () => {
  for (const mode of ["desktop", "preview"] as const) {
    selectAppRuntime(runtime(mode));
    expect(() => resolveResearchExportRuntimeGateway(browser)).toThrow(expect.objectContaining({code: "research_export_gateway_unavailable"}));
  }
  const owner = runtime("preview"); selectAppRuntime(owner); configureResearchExport(owner);
  await expect(resolveResearchExportRuntimeGateway(browser).exportAll("/synthetic"))
    .rejects.toMatchObject({code: "export_failed", message: "预览未覆盖平台调用：export_research_knowledge"});
  expect(fixture.native).not.toHaveBeenCalled();
});

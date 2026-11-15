// @vitest-environment happy-dom
import { beforeEach, expect, it, vi } from "vitest";
import type { AppRuntime } from "../../core/runtime/appRuntime";
import { configureBuiltinServices } from "./configureBuiltinServices";

const fixtures = vi.hoisted(() => ({
  literature: {get: vi.fn()}, writing: {}, transport: {}, prepared: {}, docling: {},
  getLiterature: vi.fn(), getWriting: vi.fn(), prepare: vi.fn(),
}));
vi.mock("../../core/services/aiAssistanceGateway", () => ({getAiAssistanceTransport: () => fixtures.transport}));
vi.mock("../../core/services/doclingGateway", () => ({getDoclingGateway: () => fixtures.docling}));
vi.mock("@research-assistant/literature-library-plugin/literatureGateway", () => ({getLiteratureGateway: fixtures.getLiterature}));
vi.mock("@research-assistant/writing-projects-plugin/writingGateway", () => ({getWritingGateway: fixtures.getWriting}));
vi.mock("@research-assistant/literature-library-plugin/paperPreparation", () => ({createPreparedPaperGenerationGateway: fixtures.prepare}));

beforeEach(() => {
  vi.clearAllMocks();
  fixtures.getLiterature.mockReturnValue(fixtures.literature);
  fixtures.getWriting.mockReturnValue(fixtures.writing);
  fixtures.prepare.mockReturnValue(fixtures.prepared);
});
const runtime = (mode: AppRuntime["mode"] = "desktop"): AppRuntime => ({mode, storage: window.localStorage, gateways: {}, dispose() {}});

it("retains selected service identities and one admission owner across repeated assembly", async () => {
  const selected = runtime();
  fixtures.literature.get.mockResolvedValue({title: "原文献", attachments: [{id: "attachment", displayName: "source.pdf"}]});
  configureBuiltinServices(selected);
  configureBuiltinServices(selected);
  expect(selected.gateways.getLiteratureGateway).toBe(fixtures.literature);
  expect(selected.gateways.getWritingGateway).toBe(fixtures.writing);
  expect(selected.gateways.getAiAssistanceGateway).toBe(fixtures.prepared);
  expect(fixtures.prepare).toHaveBeenCalledTimes(1);
  expect(fixtures.prepare.mock.calls[0]?.[0]).toBe(fixtures.transport);
  await expect(selected.gateways.doclingAttachmentMetadata!.describe("paper", "attachment"))
    .resolves.toEqual({paperTitle: "原文献", fileName: "source.pdf"});
  await expect(selected.gateways.doclingAttachmentMetadata!.describe("paper", "missing"))
    .resolves.toEqual({paperTitle: "原文献", fileName: "PDF"});
});

it("leaves full mock admission and gateway selection under their existing owner", () => {
  const selected = runtime("preview");
  const overrides = {...selected.gateways};
  configureBuiltinServices(selected);
  expect(selected.gateways).toEqual(overrides);
  expect(fixtures.prepare).not.toHaveBeenCalled();
  expect(fixtures.getLiterature).not.toHaveBeenCalled();
});

it("keeps native Agent preview startup within its declared capability set", () => {
  const selected = {...runtime("preview"), nativeAgentPreview: {} as NonNullable<AppRuntime["nativeAgentPreview"]>};
  configureBuiltinServices(selected);
  expect(fixtures.getLiterature).not.toHaveBeenCalled();
  expect(selected.gateways.getLiteratureGateway).toBeUndefined();
  expect(selected.gateways.doclingAttachmentMetadata).toBeUndefined();
  expect(selected.gateways.getAiAssistanceGateway).toBe(fixtures.prepared);
});

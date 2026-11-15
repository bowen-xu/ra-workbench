// @vitest-environment happy-dom
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {emptyCollectionState} from "@research-assistant/core/collections";
import {emptyNalState} from "@research-assistant/core/nal";
import {selectAppRuntime, type AppRuntime} from "../../../core/runtime/appRuntime";
import * as selected from "../../../core/public/literatureRuntime";
import {getLiteratureGateway} from "@research-assistant/literature-library-plugin/literatureGateway";
import {getCollectionGateway} from "@research-assistant/literature-library-plugin/collectionGateway";
import {createNalGateway, createNalQueryGateway} from "@research-assistant/literature-library-plugin/runtimeAdapters";
import {configureBuiltinServices} from "../configureBuiltinServices";
import {tauriNalGateway, tauriNalQueryGateway} from "./tauriCollectionNalGateways";

const fixture = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("@tauri-apps/api/core", () => ({invoke: fixture.invoke, isTauri: () => false}));
vi.mock("@research-assistant/literature-library-plugin/paperPreparation", () => ({createPreparedPaperGenerationGateway: () => ({})}));
vi.mock("../../../core/services/aiAssistanceGateway", () => ({getAiAssistanceTransport: () => ({})}));
vi.mock("../../../core/services/doclingGateway", () => ({getDoclingGateway: () => ({})}));

function runtime(mode: AppRuntime["mode"]): AppRuntime {
  const values = new Map<string, string>();
  const storage: Storage = {get length() {return values.size;}, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => {values.set(key, value);},
    removeItem: key => {values.delete(key);}, clear: () => values.clear()};
  return {mode, storage, gateways: {}, dispose() {}};
}
const readPorts = [
  ["getLiteratureGateway", selected.resolveLiteratureRuntimeGateway],
  ["getCollectionGateway", selected.resolveCollectionRuntimeGateway],
  ["getNalGateway", selected.resolveNalRuntimeGateway],
  ["getNalQueryGateway", selected.resolveNalQueryRuntimeGateway],
  ["getResearchKnowledgeGateway", selected.resolveKnowledgeRuntimeGateway],
  ["getEvidenceGateway", selected.resolveEvidenceRuntimeGateway],
  ["getEvidenceMatrixGateway", selected.resolveEvidenceMatrixRuntimeGateway],
  ["getDerivedResearchViewGateway", selected.resolveDerivedViewRuntimeGateway],
  ["getReferenceRecordGateway", selected.resolveReferenceRecordRuntimeGateway],
] as const;
beforeEach(() => {fixture.invoke.mockReset(); selectAppRuntime(runtime("browser"));});
afterEach(() => selectAppRuntime(runtime("browser")));

it("assembles all nine desktop authorities before literature lookup and Docling use", async () => {
  const owner = runtime("desktop"); selectAppRuntime(owner); configureBuiltinServices(owner);
  const identities = readPorts.map(([key, resolve]) => {
    const port = owner.gateways[key]; expect(port).toBeDefined();
    expect(resolve({} as never)).toBe(port); return port;
  });
  expect(getLiteratureGateway()).toBe(owner.gateways.getLiteratureGateway);
  configureBuiltinServices(owner);
  expect(readPorts.map(([key]) => owner.gateways[key])).toEqual(identities);
  fixture.invoke.mockResolvedValueOnce({title: "Existing paper", attachments: [{id: "attachment", displayName: "source.pdf"}]});
  await expect(owner.gateways.doclingAttachmentMetadata!.describe("paper", "attachment"))
    .resolves.toEqual({paperTitle: "Existing paper", fileName: "source.pdf"});
  expect(fixture.invoke).toHaveBeenCalledExactlyOnceWith("get_literature_item", {id: "paper"});
});

it("preserves each explicit full-mock authority by identity", () => {
  const owner = runtime("preview");
  for (const [key] of readPorts) Object.assign(owner.gateways, {[key]: {existing: key}});
  const before = {...owner.gateways}; selectAppRuntime(owner); configureBuiltinServices(owner);
  for (const [key, resolve] of readPorts) expect(resolve({} as never)).toBe(before[key]);
  expect(owner.gateways).toEqual(before);
  expect(fixture.invoke).not.toHaveBeenCalled();
});

it("does not grant literature capabilities to native Agent preview or missing full preview", () => {
  for (const native of [false, true]) {
    const owner = runtime("preview");
    if (native) Object.assign(owner, {nativeAgentPreview: {}, invoke: fixture.invoke});
    selectAppRuntime(owner); configureBuiltinServices(owner);
    for (const [key, resolve] of readPorts) {
      expect(owner.gateways[key]).toBeUndefined();
      expect(() => resolve({} as never)).toThrow(`预览未覆盖业务网关：${key}`);
    }
  }
  expect(fixture.invoke).not.toHaveBeenCalled();
});

it("retains call-time browser/native routing for captured mixed gateways", async () => {
  const browser = runtime("browser"); selectAppRuntime(browser);
  const captured = [getCollectionGateway(), createNalGateway(tauriNalGateway), createNalQueryGateway(tauriNalQueryGateway)];
  const before = await Promise.all(captured.map(port => port.read()));
  const preview = {...browser, mode: "preview" as const, gateways: {getCollectionGateway: {
    read: vi.fn(async () => emptyCollectionState()), mutate: vi.fn(async () => emptyCollectionState()),
  }}};
  selectAppRuntime(preview);
  await expect(Promise.all(captured.map(port => port.read()))).resolves.toEqual(before);
  expect(preview.gateways.getCollectionGateway!.read).not.toHaveBeenCalled();
  const desktop = runtime("desktop"); selectAppRuntime(desktop); configureBuiltinServices(desktop);
  fixture.invoke.mockImplementation(async command => command === "read_collection_network" ? emptyCollectionState()
    : command === "read_nal_network" ? emptyNalState() : {revision: 0, definitions: [], cache: null, legacyImported: false});
  await Promise.all(captured.map(port => port.read()));
  expect(fixture.invoke.mock.calls.map(call => call[0]).sort()).toEqual(["read_collection_network", "read_nal_network", "read_nal_query_collections"].sort());
  const nativeCaptured = [desktop.gateways.getCollectionGateway!, desktop.gateways.getNalGateway!, desktop.gateways.getNalQueryGateway!];
  fixture.invoke.mockClear(); selectAppRuntime(browser);
  await expect(Promise.all(nativeCaptured.map(port => port.read()))).resolves.toEqual(before);
  expect(fixture.invoke).not.toHaveBeenCalled();
});

it("does not recurse when browser or preview retains the same domain facade", async () => {
  const owner = runtime("browser"); selectAppRuntime(owner);
  const collection = getCollectionGateway(); owner.gateways.getCollectionGateway = collection;
  expect(getCollectionGateway()).toBe(collection);
  await expect(collection.read()).resolves.toEqual(emptyCollectionState());
  selectAppRuntime({...owner, mode: "preview"});
  expect(getCollectionGateway()).toBe(collection);
  await expect(collection.read()).resolves.toEqual(emptyCollectionState());
  expect(fixture.invoke).not.toHaveBeenCalled();
});

it("keeps NAL annotation and query validation before native execution", async () => {
  const owner = runtime("desktop"); selectAppRuntime(owner); configureBuiltinServices(owner);
  await expect(owner.gateways.getNalGateway!.ensureAnnotation({targetKind: "concept", targetId: "", title: "title"})).rejects.toThrow("笔记目标或标题无效");
  await expect(owner.gateways.getNalQueryGateway!.save({definitions: [{}], cache: null, legacyImported: false} as never, 0)).rejects.toThrow();
  expect(fixture.invoke).not.toHaveBeenCalled();
});

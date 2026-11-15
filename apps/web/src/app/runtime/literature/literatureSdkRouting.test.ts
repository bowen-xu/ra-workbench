// @vitest-environment happy-dom
import {afterEach, expect, it} from "vitest";
import * as sdk from "@research-assistant/workbench-sdk/literature-runtime";
import * as core from "../../../core/public/literatureRuntime";
import {getAppRuntime, selectAppRuntime, type AppRuntime} from "../../../core/runtime/appRuntime";
import type {AppGateways} from "../../../core/runtime/appGatewayTypes";

const previous = getAppRuntime();
afterEach(() => selectAppRuntime(previous));
const runtime = (mode: AppRuntime["mode"]): AppRuntime => ({mode, storage: window.localStorage, gateways: {}, dispose() {}});

it("the SDK binds the original ten Core functions without exposing runtime or invocation", () => {
  const names = ["resolveLiteratureRuntimeGateway", "resolveCollectionRuntimeGateway", "resolveNalRuntimeGateway", "resolveNalQueryRuntimeGateway",
    "resolveKnowledgeRuntimeGateway", "resolveEvidenceRuntimeGateway", "resolveEvidenceMatrixRuntimeGateway", "resolveDerivedViewRuntimeGateway",
    "resolveReferenceRecordRuntimeGateway", "resolveResearchExportRuntimeGateway"] as const;
  expect(Object.keys(sdk).sort()).toEqual([...names].sort());
  for (const name of names) expect(sdk[name]).toBe(core[name]);
});

it("browser explicit domain ports still precede fallback while export always retains its refusal port", () => {
  const owner = runtime("browser");
  const ports = [
    ["getLiteratureGateway", sdk.resolveLiteratureRuntimeGateway],
    ["getCollectionGateway", sdk.resolveCollectionRuntimeGateway],
    ["getNalGateway", sdk.resolveNalRuntimeGateway],
    ["getNalQueryGateway", sdk.resolveNalQueryRuntimeGateway],
    ["getResearchKnowledgeGateway", sdk.resolveKnowledgeRuntimeGateway],
    ["getEvidenceGateway", sdk.resolveEvidenceRuntimeGateway],
    ["getEvidenceMatrixGateway", sdk.resolveEvidenceMatrixRuntimeGateway],
    ["getDerivedResearchViewGateway", sdk.resolveDerivedViewRuntimeGateway],
    ["getReferenceRecordGateway", sdk.resolveReferenceRecordRuntimeGateway],
  ] as const;
  // The resolver performs identity selection only; these ports are never invoked.
  for (const [key] of ports) Object.assign(owner.gateways, {[key]: {selected: key}});
  owner.gateways.getResearchExportGateway = {exportAll: async () => {throw new Error("must not execute");}, cancel: async () => false};
  selectAppRuntime(owner);
  for (const [key, resolve] of ports) expect(resolve({} as never)).toBe(owner.gateways[key]);
  const browser = {exportAll: async () => {throw new Error("browser refusal");}, cancel: async () => false};
  expect(sdk.resolveResearchExportRuntimeGateway(browser)).toBe(browser);
});

it("resolver calls select the current owner without replacing previously returned port identities", () => {
  const first = runtime("preview"), second = runtime("desktop");
  const a: AppGateways["getCollectionGateway"] = {read: async () => {throw new Error("identity only");}, mutate: async () => {throw new Error("identity only");}};
  const b: AppGateways["getCollectionGateway"] = {read: a.read, mutate: a.mutate};
  first.gateways.getCollectionGateway = a; second.gateways.getCollectionGateway = b;
  selectAppRuntime(first);
  const captured = sdk.resolveCollectionRuntimeGateway(a);
  selectAppRuntime(second);
  expect(captured).toBe(a);
  expect(sdk.resolveCollectionRuntimeGateway(a)).toBe(b);
  selectAppRuntime(runtime("preview"));
  expect(() => sdk.resolveCollectionRuntimeGateway(a)).toThrow("预览未覆盖业务网关：getCollectionGateway");
  selectAppRuntime(runtime("desktop"));
  expect(() => sdk.resolveCollectionRuntimeGateway(a)).toThrow(expect.objectContaining({code: "literature_gateway_unavailable"}));
});

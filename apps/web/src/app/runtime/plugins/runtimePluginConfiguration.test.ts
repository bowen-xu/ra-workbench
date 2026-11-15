// @vitest-environment happy-dom
import {beforeEach, expect, it} from "vitest";
import {decodePluginManifest, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {LoadedRuntimePlugin} from "../../../core/runtime/plugins/loader";
import {writingConfiguration} from "@research-assistant/writing-projects-plugin/configuration";
import {bindRuntimePluginConfigurations} from "./runtimePluginConfiguration";

const applicationKey = "research-assistant:workspace.v1:writing-outline";
const resourceKey = applicationKey + ":original-manuscript";
const scope = {kind: "resource" as const, resourceId: "original-manuscript"};
const manifest = decodePluginManifest({formatVersion: 1, id: "writing_projects", version: "1.0.0", apiVersion: 1,
  entry: "assets/entry.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME,
  dependencies: {required: [], optional: []}, provides: [], consumes: {required: [], optional: []}});
const plugin: LoadedRuntimePlugin = {manifest, entryUrl: "/plugins/fixture/assets/entry.js", styles: [],
  module: {configuration: writingConfiguration, activate: () => undefined}};

beforeEach(() => localStorage.clear());

function bindWriting() {
  const configuration = bindRuntimePluginConfigurations(localStorage, [plugin]).get("writing_projects");
  if (!configuration) throw new Error("Writing configuration missing");
  return configuration;
}

it("keeps the original writing application and manuscript keys across runtime binding and reopening", () => {
  localStorage.setItem(applicationKey, "false");
  localStorage.setItem(resourceKey, "true");
  const configuration = bindWriting();
  expect(configuration.get("outlineVisible")).toBe(false);
  expect(configuration.get("outlineVisible", scope)).toBe(true);
  expect(configuration.inspect("outlineVisible", scope).origin).toBe("resource");
  configuration.update("outlineVisible", false, scope);
  expect(localStorage.getItem(resourceKey)).toBe("false");
  configuration.update("outlineVisible", true);
  expect(localStorage.getItem(applicationKey)).toBe("true");
  expect(configuration.get("outlineVisible", scope)).toBe(false);
  expect(bindWriting().get("outlineVisible", scope)).toBe(false);
  configuration.update("outlineVisible", undefined, scope);
  expect(localStorage.getItem(resourceKey)).toBeNull();
  expect(bindWriting().get("outlineVisible", scope)).toBe(true);
  expect(localStorage.getItem("research-assistant:runtime-plugin.v1:writing_projects:outlineVisible")).toBeNull();
});

it("preserves invalid original resource bytes while using the valid original application fallback", () => {
  localStorage.setItem(applicationKey, "false");
  localStorage.setItem(resourceKey, '"legacy-invalid"');
  const configuration = bindWriting();
  expect(configuration.get("outlineVisible", scope)).toBe(false);
  expect(configuration.inspect("outlineVisible", scope).invalidScopes).toContain("resource");
  expect(localStorage.getItem(resourceKey)).toBe('"legacy-invalid"');
  configuration.update("outlineVisible", true, scope);
  expect(bindWriting().get("outlineVisible", scope)).toBe(true);
  expect(localStorage.getItem(applicationKey)).toBe("false");
});

import {describe, expect, it, vi} from "vitest";
import {createPluginManagementGateway} from "./pluginManagementGateway";
import {WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";

const key = "a".repeat(64), revision = "b".repeat(64);
function plugin(id = "note_manager", enabled = false, required: {id: string; apiVersion: number}[] = []) {
  return {package: {packageKey: key, manifest: {formatVersion: 1, id, version: "0.1.0", apiVersion: 1,
    entry: "index.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME,
    dependencies: {required, optional: []}, consumes: {required: [], optional: []}, provides: []}},
  enabled, removed: false, startupPackageKey: null, origin: "installed"};
}
function fixture(rows = [plugin()]) {
  const invoke = vi.fn(async (command: string): Promise<unknown> => command === "list_managed_plugins" ? {revision, plugins: rows} : undefined);
  const choosePackage = vi.fn(async (): Promise<string | null> => "/synthetic/manifest.json");
  const gateway = createPluginManagementGateway({coreCapabilities: [], readRuntime: () => [], isInitialLoadComplete: () => true}, {invoke, choosePackage});
  return {gateway, invoke, choosePackage, rows};
}

describe("local plugin management adapter", () => {
  it("uses the persisted revision, and enabling affects the next startup", async () => {
    const {gateway, invoke, rows} = fixture();
    expect((await gateway.list())[0]?.runtime).toBe("disabled");
    await gateway.setEnabled("note_manager", true);
    expect(invoke).toHaveBeenLastCalledWith("set_managed_plugin_enabled", {pluginId: "note_manager", enabled: true, expectedRevision: revision});
    rows[0] = {...plugin(), enabled: true};
    expect((await gateway.list())[0]?.runtime).toBe("restart");
  });
  it("rejects enabling a plugin with a missing required dependency before mutation", async () => {
    const row = plugin("note_manager", false, [{id: "missing", apiVersion: 1}]);
    const {gateway, invoke} = fixture([row]);
    await gateway.list();
    await expect(gateway.setEnabled("note_manager", true)).rejects.toThrow();
    expect(invoke.mock.calls.map(call => call[0])).toEqual(["list_managed_plugins"]);
  });
  it("rejects disabling a required provider without silently disabling consumers", async () => {
    const consumer = plugin("consumer", true, [{id: "note_manager", apiVersion: 1}]);
    const {gateway, invoke} = fixture([plugin("note_manager", true), consumer]);
    await gateway.list();
    await expect(gateway.setEnabled("note_manager", false)).rejects.toThrow();
    expect(invoke.mock.calls.map(call => call[0])).toEqual(["list_managed_plugins"]);
  });
  it("cancels package picking without invoking an installation", async () => {
    const {gateway, invoke, choosePackage} = fixture();
    await gateway.list();
    choosePackage.mockResolvedValueOnce(null);
    expect(await gateway.importPackage()).toBeNull();
    expect(invoke.mock.calls.map(call => call[0])).toEqual(["list_managed_plugins"]);
  });
  it("installs the selected source and reads the actual resulting record", async () => {
    const {gateway, invoke, rows} = fixture();
    await gateway.list();
    invoke.mockImplementation(async command => {
      if (command === "install_managed_plugin") return rows[0]?.package;
      return {revision, plugins: rows};
    });
    expect((await gateway.importPackage())?.id).toBe("note_manager");
    expect(invoke).toHaveBeenCalledWith("install_managed_plugin", {source: "/synthetic/manifest.json", expectedRevision: revision});
  });
  it("preserves the stable native conflict diagnostic", async () => {
    const {gateway, invoke} = fixture();
    await gateway.list();
    invoke.mockRejectedValueOnce({code: "plugin_registry_conflict", message: "请刷新后重试"});
    await expect(gateway.uninstall("note_manager")).rejects.toThrow("plugin_registry_conflict：请刷新后重试");
  });
});

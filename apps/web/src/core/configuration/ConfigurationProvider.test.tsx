// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {notify} from "../services/notifications";
import type {ConfigurationSetting} from "../contracts/configuration";
import {createPluginConfigurations} from "./configuration";
import {PluginConfigurationProvider, usePluginSetting} from "./ConfigurationProvider";
vi.mock("../services/notifications", () => ({notify: {error: vi.fn()}}));

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const setting: ConfigurationSetting<boolean> = {key: "visible", defaultValue: true, schema: {type: "boolean"}, scopes: ["application", "resource"], effect: "immediate"};
function Consumer({name, resourceId, enabled = true}: {readonly name: string; readonly resourceId?: string; readonly enabled?: boolean}) {
  const [value, update] = usePluginSetting(setting, {enabled, ...(resourceId === undefined ? {} : {resourceId})});
  return <button data-consumer={name} onClick={() => update(previous => !previous)}>{String(value)}</button>;
}

it("shares validated changes between consumers, preserves local embedded state and switches resource without writing old state", async () => {
  localStorage.clear();
  const configuration = createPluginConfigurations(localStorage, [{owner: "fixture", declaration: {owner: "fixture", settings: [setting]}, storage: {visible: {application: "existing", resourcePrefix: "existing:"}}}]).get("fixture");
  if (!configuration) throw new Error("Fixture owner missing");
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  const button = (name: string) => host.querySelector<HTMLButtonElement>(`[data-consumer="${name}"]`);
  const render = (resourceId = "one") => <PluginConfigurationProvider value={configuration}><Consumer name="first" resourceId={resourceId}/><Consumer name="second" resourceId={resourceId}/><Consumer name="embedded" enabled={false}/></PluginConfigurationProvider>;
  try {
    await act(async () => root.render(render()));
    expect(button("first")?.textContent).toBe("true");
    await act(async () => button("first")?.click());
    expect(button("second")?.textContent).toBe("false");
    expect(button("embedded")?.textContent).toBe("true");
    await act(async () => button("embedded")?.click());
    expect(localStorage.getItem("existing")).toBeNull();
    await act(async () => root.render(render("two")));
    expect(button("first")?.textContent).toBe("true");
    expect(localStorage.getItem("existing:two")).toBeNull();
    expect(localStorage.getItem("existing:one")).toBe("false");
    await act(async () => configuration.update("visible", false));
    expect(button("first")?.textContent).toBe("false");
    expect(button("second")?.textContent).toBe("false");
    expect(button("embedded")?.textContent).toBe("false");
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("keeps the UI and original bytes through denied reads, reports outside render and updates after recovery", async () => {
  let denied = true, rendering = false;
  const values = new Map([["existing", "false"]]), write = vi.fn((key: string, value: string) => {values.set(key, value);});
  const storage: Storage = {get length() {return values.size;}, key: index => [...values.keys()][index] ?? null,
    getItem: key => {if (denied) throw new DOMException("synthetic-private-value", "SecurityError"); return values.get(key) ?? null;},
    setItem: write, removeItem: key => {values.delete(key);}, clear: () => {values.clear();}};
  const configuration = createPluginConfigurations(storage, [{owner: "fixture", declaration: {owner: "fixture", settings: [setting]}, storage: {visible: {application: "existing", resourcePrefix: "existing:"}}}]).get("fixture");
  if (!configuration) throw new Error("Fixture owner missing");
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  vi.mocked(notify.error).mockClear();
  vi.mocked(notify.error).mockImplementation(() => {expect(rendering).toBe(false); return "test-notification";});
  function DeniedReader() {
    rendering = true;
    const [value, update] = usePluginSetting(setting);
    rendering = false;
    return <button onClick={() => update(previous => !previous)}>{String(value)}</button>;
  }
  try {
    await act(async () => root.render(<PluginConfigurationProvider value={configuration}><DeniedReader/></PluginConfigurationProvider>));
    expect(host.querySelector("button")?.textContent).toBe("true");
    expect(write).not.toHaveBeenCalled(); expect(values.get("existing")).toBe("false");
    expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("SecurityError"));
    expect(JSON.stringify(vi.mocked(notify.error).mock.calls)).not.toContain("synthetic-private-value");
    denied = false;
    await act(async () => window.dispatchEvent(new StorageEvent("storage", {key: "existing"})));
    expect(host.querySelector("button")?.textContent).toBe("false");
    await act(async () => host.querySelector<HTMLButtonElement>("button")?.click());
    expect(host.querySelector("button")?.textContent).toBe("true");
    expect(values.get("existing")).toBe("true"); expect(write).toHaveBeenCalledTimes(1);
    denied = true;
    await act(async () => window.dispatchEvent(new StorageEvent("storage", {key: "existing"})));
    expect(host.querySelector("button")?.textContent).toBe("true");
    expect(write).toHaveBeenCalledTimes(1);
    expect(JSON.stringify(vi.mocked(notify.error).mock.calls)).not.toContain("synthetic-private-value");
  } finally { await act(async () => root.unmount()); host.remove(); vi.mocked(notify.error).mockReset(); }
});

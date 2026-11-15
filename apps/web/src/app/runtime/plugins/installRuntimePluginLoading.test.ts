// @vitest-environment happy-dom
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import type {GlobalWindow} from "happy-dom";
import {decodePluginManifest, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {LoadedRuntimePlugin, RuntimePluginLoadResult} from "../../../core/runtime/plugins/loader";
import {installRuntimePluginLoading} from "./installRuntimePluginLoading";
import type {InstalledRuntimePluginLoading, RuntimePluginLoadingSummary, RuntimePluginLoadingOptions} from "./installRuntimePluginLoading";

const cleanups: (() => void)[] = [];
const browser = window as unknown as GlobalWindow;
const originalInterceptor = browser.happyDOM.settings.fetch.interceptor;
const networkReleases: (() => void)[] = [];
beforeEach(() => {
  document.head.replaceChildren();
  // Keep real link elements and production event ownership, while replacing
  // the environment's network effect. Each test controls load/error timing.
  browser.happyDOM.settings.fetch.interceptor = {
    beforeAsyncRequest: ({window}) => new Promise(resolve => {
      networkReleases.push(() => resolve(new window.Response("")));
    }),
  };
});
afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) cleanup();
  for (const release of networkReleases.splice(0)) release();
  vi.useRealTimers();
  await browser.happyDOM.waitUntilComplete();
  browser.happyDOM.settings.fetch.interceptor = originalInterceptor;
});

function plugin(id = "note_manager", index = 1, files: readonly string[] = ["notes.css"]): LoadedRuntimePlugin {
  const key = index.toString(16).padStart(64, "0");
  const manifest = decodePluginManifest({
    formatVersion: 1, id, version: "1.0.0", apiVersion: 1, entry: "assets/entry.js",
    styles: files.map(file => `assets/${file}`), uiRuntime: WORKBENCH_UI_RUNTIME,
    dependencies: {required: [], optional: []}, provides: [], consumes: {required: [], optional: []},
  });
  return {manifest, module: {activate: vi.fn()}, entryUrl: `/plugins/${key}/${manifest.entry}`,
    styles: manifest.styles.map(path => `/plugins/${key}/${path}`)};
}
function result(...plugins: readonly LoadedRuntimePlugin[]): RuntimePluginLoadResult {
  return {plugins, bindings: [], unavailableOptional: []};
}
function setup(value = result(plugin()), options: Pick<RuntimePluginLoadingOptions, "retainPluginStyles" | "onDeactivate"> = {}) {
  const loader = {load: vi.fn(async (): Promise<RuntimePluginLoadResult> => value)};
  const onError = vi.fn(), onLoaded = vi.fn();
  const installed = installRuntimePluginLoading({loader, onError, onLoaded, target: window, document, ...options});
  cleanups.push(() => installed.dispose());
  return {loader, onError, onLoaded, installed};
}
function links(): HTMLLinkElement[] {
  return [...document.head.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"]')];
}
function broadcasts(eventName = "research-assistant:runtime-plugins-loaded") {
  const received = vi.fn();
  window.addEventListener(eventName, received);
  cleanups.push(() => window.removeEventListener(eventName, received));
  return received;
}
async function finishStyles(installed: InstalledRuntimePluginLoading): Promise<RuntimePluginLoadResult> {
  const pending = installed.load();
  await Promise.resolve();
  for (const link of links()) link.dispatchEvent(new Event("load"));
  return pending;
}

it("waits for the actual stylesheet load event before callback and safe summary broadcast", async () => {
  const value = result(plugin());
  const state = setup(value), received = broadcasts();
  const pending = state.installed.load();
  await Promise.resolve();
  expect(links().map(link => link.getAttribute("href"))).toEqual(value.plugins[0]?.styles);
  expect(state.onLoaded).not.toHaveBeenCalled();
  expect(received).not.toHaveBeenCalled();
  links()[0]?.dispatchEvent(new Event("load"));
  expect(await pending).toBe(value);
  expect(state.onLoaded).toHaveBeenCalledExactlyOnceWith(value);
  const event = received.mock.calls[0]?.[0] as CustomEvent<RuntimePluginLoadingSummary>;
  expect(event.detail).toEqual({plugins: [{id: "note_manager", version: "1.0.0",
    entryUrl: value.plugins[0]?.entryUrl, styles: value.plugins[0]?.styles}], bindings: [], unavailableOptional: []});
  expect(JSON.stringify(event.detail)).not.toContain("module");
  expect(event.detail.plugins[0]).not.toHaveProperty("manifest");
  expect(state.onError).not.toHaveBeenCalled();
});

it("waits for asynchronous activation before broadcasting or releasing prior styles", async () => {
  const state = setup(), received = broadcasts();
  await finishStyles(state.installed);
  received.mockClear();
  const previous = links()[0];
  const replacement = result(plugin("note_manager", 2));
  state.loader.load.mockResolvedValueOnce(replacement);
  let activate!: () => void;
  state.onLoaded.mockImplementationOnce(() => new Promise<void>(resolve => {activate = resolve;}));
  let settled = false;
  const pending = state.installed.load().finally(() => {settled = true;});
  await Promise.resolve();
  links().find(link => link !== previous)?.dispatchEvent(new Event("load"));
  await vi.waitFor(() => expect(state.onLoaded).toHaveBeenCalledTimes(2));
  expect(received).not.toHaveBeenCalled();
  expect(settled).toBe(false);
  expect(previous?.isConnected).toBe(true);
  activate();
  expect(await pending).toBe(replacement);
  expect(previous?.isConnected).toBe(false);
  expect(links()).toHaveLength(1);
  expect(received).toHaveBeenCalledTimes(1);
});

it("rolls back only new styles after an asynchronous activation rejection", async () => {
  const state = setup(), received = broadcasts();
  await finishStyles(state.installed);
  received.mockClear();
  const previous = links()[0], cause = new Error("plugin_activation_failed: note_manager");
  state.loader.load.mockResolvedValueOnce(result(plugin("note_manager", 2)));
  state.onLoaded.mockImplementationOnce(async () => {throw cause;});
  const pending = state.installed.load();
  const rejected = expect(pending).rejects.toBe(cause);
  await Promise.resolve();
  links().find(link => link !== previous)?.dispatchEvent(new Event("load"));
  await rejected;
  expect(links()).toEqual([previous]);
  expect(received).not.toHaveBeenCalled();
});

it("keeps ready styles for a successful new instance when a later activation fails", async () => {
  const first = plugin(), failed = plugin("second_fixture", 2);
  const active = new Set<string>();
  const state = setup(result(first, failed), {retainPluginStyles: entryUrl => active.has(entryUrl)});
  const cause = new Error("plugin_activation_failed: second_fixture");
  state.onLoaded.mockImplementationOnce(async () => {active.add(first.entryUrl); throw cause;});
  const pending = state.installed.load(), rejected = expect(pending).rejects.toBe(cause);
  await Promise.resolve();
  for (const link of links()) link.dispatchEvent(new Event("load"));
  await rejected;
  expect(links().map(link => link.getAttribute("href"))).toEqual(first.styles);
});

it("does not retain a rejected replacement's styles because the prior package ID is active", async () => {
  const previous = plugin(), replacement = plugin("note_manager", 2);
  const state = setup(result(previous), {retainPluginStyles: entryUrl => entryUrl === previous.entryUrl});
  await finishStyles(state.installed);
  state.loader.load.mockResolvedValueOnce(result(replacement));
  const cause = new Error("plugin_restart_required: note_manager");
  state.onLoaded.mockRejectedValueOnce(cause);
  const pending = state.installed.load(), rejected = expect(pending).rejects.toBe(cause);
  await Promise.resolve();
  for (const link of links()) link.dispatchEvent(new Event("load"));
  await rejected;
  expect(links().map(link => link.getAttribute("href"))).toEqual(previous.styles);
});

it("cannot broadcast or remount styles when disposal happens during asynchronous activation", async () => {
  const state = setup(undefined, {retainPluginStyles: () => true}), received = broadcasts();
  let activate!: () => void;
  state.onLoaded.mockImplementationOnce(() => new Promise<void>(resolve => {activate = resolve;}));
  const pending = state.installed.load();
  const rejected = expect(pending).rejects.toMatchObject({code: "plugin_loading_disposed"});
  await Promise.resolve();
  links()[0]?.dispatchEvent(new Event("load"));
  await vi.waitFor(() => expect(state.onLoaded).toHaveBeenCalledTimes(1));
  state.installed.dispose();
  activate();
  await rejected;
  expect(links()).toEqual([]);
  expect(received).not.toHaveBeenCalled();
});

it("coalesces concurrent load calls and reuses already ready styles on another scan", async () => {
  const state = setup();
  const first = state.installed.load(), concurrent = state.installed.load();
  expect(first).toBe(concurrent);
  await Promise.resolve();
  const original = links()[0];
  original?.dispatchEvent(new Event("load"));
  await first;
  await state.installed.load();
  expect(state.loader.load).toHaveBeenCalledTimes(2);
  expect(links()).toEqual([original]);
});

it("deduplicates a shared href within one result without changing plugin manifests", async () => {
  const first = plugin(), second = {...plugin("other", 2), styles: first.styles};
  const state = setup(result(first, second));
  await finishStyles(state.installed);
  expect(links()).toHaveLength(1);
});

it("leaves direct Core loading rejections to their caller without attaching CSS or reporting twice", async () => {
  const state = setup(), received = broadcasts();
  const cause = Object.assign(new Error("rejected manifest"), {code: "plugin_api_incompatible"});
  state.loader.load.mockRejectedValueOnce(cause);
  await expect(state.installed.load()).rejects.toBe(cause);
  expect(links()).toEqual([]);
  expect(state.onError).not.toHaveBeenCalled();
  expect(state.onLoaded).not.toHaveBeenCalled();
  expect(received).not.toHaveBeenCalled();
});

it("uses the same load for an internal event and returns only the summary receipt", async () => {
  const state = setup(result()), received = broadcasts();
  const resolve = vi.fn(), reject = vi.fn();
  window.dispatchEvent(new CustomEvent("research-assistant:load-runtime-plugins", {detail: {resolve, reject}}));
  await state.installed.load();
  await Promise.resolve();
  expect(state.loader.load).toHaveBeenCalledTimes(1);
  expect(resolve).toHaveBeenCalledExactlyOnceWith({plugins: [], bindings: [], unavailableOptional: []});
  expect(reject).not.toHaveBeenCalled();
  expect(received).toHaveBeenCalledTimes(1);
});

it("reports a failed shared event operation once and rejects each typed receipt", async () => {
  const state = setup();
  const cause = Object.assign(new Error("missing required dependency"), {code: "plugin_dependency_missing"});
  state.loader.load.mockRejectedValueOnce(cause);
  const rejectFirst = vi.fn(), rejectSecond = vi.fn();
  window.dispatchEvent(new CustomEvent("research-assistant:load-runtime-plugins", {detail: {reject: rejectFirst}}));
  window.dispatchEvent(new CustomEvent("research-assistant:load-runtime-plugins", {detail: {reject: rejectSecond}}));
  await expect(state.installed.load()).rejects.toBe(cause);
  expect(state.onError).toHaveBeenCalledExactlyOnceWith(cause);
  expect(rejectFirst).toHaveBeenCalledExactlyOnceWith(cause);
  expect(rejectSecond).toHaveBeenCalledExactlyOnceWith(cause);
});

it.each([undefined, null, "invalid", {resolve: true, reject: 42}])(
  "checks optional event callbacks before calling them: %j", async detail => {
    const state = setup(result());
    window.dispatchEvent(new CustomEvent("research-assistant:load-runtime-plugins", {detail}));
    await state.installed.load();
    expect(state.onLoaded).toHaveBeenCalledTimes(1);
    expect(state.onError).not.toHaveBeenCalled();
  },
);

it("rolls back this scan's successful and failed CSS while retaining prior successful links", async () => {
  const state = setup();
  await finishStyles(state.installed);
  const prior = links()[0];
  state.loader.load.mockResolvedValueOnce(result(plugin(), plugin("other", 2, ["first.css", "second.css"])));
  const pending = state.installed.load();
  await Promise.resolve();
  expect(links()).toHaveLength(3);
  links()[1]?.dispatchEvent(new Event("load"));
  const rejected = expect(pending).rejects.toMatchObject({code: "plugin_stylesheet_failed", diagnostic: {
    pluginId: "other", href: plugin("other", 2, ["second.css"]).styles[0],
  }});
  links()[2]?.dispatchEvent(new Event("error"));
  await rejected;
  expect(links()).toEqual([prior]);
  expect(state.onLoaded).toHaveBeenCalledTimes(1);
  expect(state.onError).not.toHaveBeenCalled();
});

it("releases the previous hash's styles only once the complete replacement succeeds", async () => {
  const state = setup();
  await finishStyles(state.installed);
  const prior = links()[0];
  const replacement = plugin("note_manager", 2);
  state.loader.load.mockResolvedValueOnce(result(replacement));
  const pending = state.installed.load();
  await Promise.resolve();
  const next = links()[1];
  expect(links()).toEqual([prior, next]);
  expect(prior?.isConnected).toBe(true);
  next?.dispatchEvent(new Event("load"));
  await pending;
  expect(prior?.isConnected).toBe(false);
  expect(links()).toEqual([next]);
  expect(next?.getAttribute("href")).toBe(replacement.styles[0]);
});

it("releases all prior plugin CSS when an empty catalog scan succeeds", async () => {
  const state = setup();
  await finishStyles(state.installed);
  const prior = links()[0];
  state.loader.load.mockResolvedValueOnce(result());
  await state.installed.load();
  expect(prior?.isConnected).toBe(false);
  expect(links()).toEqual([]);
  expect(state.onLoaded).toHaveBeenCalledTimes(2);
});

it("preserves the old hash's styles if the ready replacement's onLoaded callback fails", async () => {
  const state = setup();
  await finishStyles(state.installed);
  const prior = links()[0];
  const cause = new Error("App rejected the replacement");
  state.loader.load.mockResolvedValueOnce(result(plugin("note_manager", 2)));
  state.onLoaded.mockImplementationOnce(() => { throw cause; });
  const pending = state.installed.load();
  await Promise.resolve();
  const rejected = expect(pending).rejects.toBe(cause);
  links()[1]?.dispatchEvent(new Event("load"));
  await rejected;
  expect(links()).toEqual([prior]);
  expect(prior?.isConnected).toBe(true);
});

it("bounds a missing load/error event, removes its timer/link and allows an explicit retry", async () => {
  vi.useFakeTimers();
  const state = setup();
  const pending = state.installed.load();
  await Promise.resolve();
  const rejected = expect(pending).rejects.toMatchObject({code: "plugin_stylesheet_timeout", diagnostic: {
    pluginId: "note_manager", timeoutMs: 30_000,
  }});
  await vi.advanceTimersByTimeAsync(30_000);
  await rejected;
  expect(links()).toEqual([]);
  expect(vi.getTimerCount()).toBe(0);
  await finishStyles(state.installed);
  expect(state.onLoaded).toHaveBeenCalledTimes(1);
  expect(vi.getTimerCount()).toBe(0);
});

it("clears pending CSS, timers and the request listener on dispose; late events cannot complete a load", async () => {
  vi.useFakeTimers();
  const state = setup(), received = broadcasts();
  const pending = state.installed.load();
  await Promise.resolve();
  const link = links()[0];
  const rejected = expect(pending).rejects.toMatchObject({code: "plugin_loading_disposed"});
  state.installed.dispose();
  state.installed.dispose();
  await rejected;
  link?.dispatchEvent(new Event("load"));
  window.dispatchEvent(new Event("research-assistant:load-runtime-plugins"));
  await Promise.resolve();
  expect(links()).toEqual([]);
  expect(vi.getTimerCount()).toBe(0);
  expect(state.loader.load).toHaveBeenCalledTimes(1);
  expect(state.onLoaded).not.toHaveBeenCalled();
  expect(received).not.toHaveBeenCalled();
  await expect(state.installed.load()).rejects.toMatchObject({code: "plugin_loading_disposed"});
});

it("never attaches styles for a Core module result that arrives after disposal", async () => {
  const state = setup(), received = broadcasts();
  let complete: ((value: RuntimePluginLoadResult) => void) | undefined;
  state.loader.load.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  const pending = state.installed.load();
  state.installed.dispose();
  if (!complete) throw new Error("Core loading did not start");
  complete(result(plugin()));
  await expect(pending).rejects.toMatchObject({code: "plugin_loading_disposed"});
  expect(links()).toEqual([]);
  expect(state.onLoaded).not.toHaveBeenCalled();
  expect(received).not.toHaveBeenCalled();
});

it("disposes only its own successful styles and preserves another App stylesheet", async () => {
  const foreign = document.createElement("link");
  foreign.rel = "stylesheet"; foreign.href = "/app.css";
  document.head.append(foreign);
  const state = setup();
  await finishStyles(state.installed);
  state.installed.dispose();
  expect(links()).toEqual([foreign]);
});

function deferred() {
  let resolve: () => void = () => { throw Error("Deferred executor missing"); };
  const promise = new Promise<void>(accept => { resolve = accept; });
  return {promise, resolve};
}

it("awaits safe stop once, releases stopped dependents and preserves another exact owner's shared href", async () => {
  const first = plugin(), dependent = plugin("dependent", 2), other = plugin("other", 3);
  const shared = {...other, styles: [...first.styles, ...other.styles]};
  const active = new Set([first.entryUrl, dependent.entryUrl, other.entryUrl]), gate = deferred();
  const onDeactivate = vi.fn(async () => { await gate.promise; active.delete(first.entryUrl); active.delete(dependent.entryUrl); });
  const state = setup(result(first, dependent, shared), {onDeactivate, retainPluginStyles: entry => active.has(entry)});
  const received = broadcasts("research-assistant:runtime-plugin-deactivated");
  await finishStyles(state.installed);
  const original = links(), stopping = state.installed.deactivate("note_manager");
  expect(state.installed.deactivate("note_manager")).toBe(stopping);
  await vi.waitFor(() => expect(onDeactivate).toHaveBeenCalledExactlyOnceWith("note_manager"));
  expect(links()).toEqual(original); expect(received).not.toHaveBeenCalled();
  gate.resolve(); await stopping;
  expect(links().map(link => link.getAttribute("href"))).toEqual([...first.styles, ...other.styles]);
  expect(original.find(link => link.getAttribute("href") === dependent.styles[0])?.isConnected).toBe(false);
  expect(received).toHaveBeenCalledOnce();
});

it.each(["plugin_view_close_blocked", "plugin_lifecycle_timeout"])("keeps every ready stylesheet when safe stop rejects with %s", async code => {
  const cause = Object.assign(Error(code), {code, pluginId: "note_manager"});
  const onDeactivate = vi.fn(async () => { throw cause; });
  const state = setup(undefined, {onDeactivate, retainPluginStyles: () => true});
  const received = broadcasts("research-assistant:runtime-plugin-deactivated");
  await finishStyles(state.installed); const original = links();
  await expect(state.installed.deactivate("note_manager")).rejects.toBe(cause);
  expect(links()).toEqual(original); expect(received).not.toHaveBeenCalled();
  expect(state.onError).not.toHaveBeenCalled();
  const rejectFirst = vi.fn(), rejectSecond = vi.fn();
  for (const reject of [rejectFirst, rejectSecond])
    window.dispatchEvent(new CustomEvent("research-assistant:deactivate-runtime-plugin", {detail: {pluginId: "note_manager", reject}}));
  await expect(state.installed.deactivate("note_manager")).rejects.toBe(cause);
  await Promise.resolve();
  expect(state.onError).toHaveBeenCalledExactlyOnceWith(cause);
  expect(rejectFirst).toHaveBeenCalledExactlyOnceWith(cause); expect(rejectSecond).toHaveBeenCalledExactlyOnceWith(cause);
});

it("validates the actual loaded owner, handles CSS-free packages, and refuses an unavailable or incomplete stop", async () => {
  const empty = plugin("empty", 1, []), active = new Set([empty.entryUrl]);
  const onDeactivate = vi.fn(async () => { active.delete(empty.entryUrl); });
  const state = setup(result(empty), {onDeactivate, retainPluginStyles: entry => active.has(entry)});
  await state.installed.load();
  await expect(state.installed.deactivate("unknown")).rejects.toMatchObject({code: "plugin_unknown_instance", diagnostic: {pluginId: "unknown"}});
  expect(onDeactivate).not.toHaveBeenCalled();
  await state.installed.deactivate("empty"); expect(onDeactivate).toHaveBeenCalledExactlyOnceWith("empty");
  const unavailable = setup(); await finishStyles(unavailable.installed); const original = links();
  await expect(unavailable.installed.deactivate("note_manager")).rejects.toMatchObject({code: "plugin_deactivation_unavailable"});
  expect(links()).toEqual(original);
  const incomplete = setup(result(plugin("retained", 2)), {onDeactivate: vi.fn(async () => undefined), retainPluginStyles: () => true});
  await finishStyles(incomplete.installed); const retained = links();
  await expect(incomplete.installed.deactivate("retained")).rejects.toMatchObject({code: "plugin_deactivation_incomplete"});
  expect(links()).toEqual(retained);
});

it("returns a minimal deactivation receipt through the internal event and coalesces its duplicate request", async () => {
  const active = new Set([plugin().entryUrl]), gate = deferred();
  const state = setup(undefined, {retainPluginStyles: entry => active.has(entry),
    onDeactivate: async () => { await gate.promise; active.clear(); }});
  await finishStyles(state.installed);
  const resolve = vi.fn(), second = vi.fn(), reject = vi.fn();
  window.dispatchEvent(new CustomEvent("research-assistant:deactivate-runtime-plugin", {detail: {pluginId: "note_manager", resolve, reject}}));
  window.dispatchEvent(new CustomEvent("research-assistant:deactivate-runtime-plugin", {detail: {pluginId: "note_manager", resolve: second, reject}}));
  const pending = state.installed.deactivate("note_manager"); gate.resolve(); await pending; await Promise.resolve();
  expect(resolve).toHaveBeenCalledExactlyOnceWith({pluginId: "note_manager"});
  expect(second).toHaveBeenCalledExactlyOnceWith({pluginId: "note_manager"});
  expect(reject).not.toHaveBeenCalled(); expect(links()).toEqual([]);
  expect(state.onError).not.toHaveBeenCalled();
});

it.each([undefined, null, [], "../escape", "core"])("rejects an invalid stop event ID without calling App: %j", async pluginId => {
  const onDeactivate = vi.fn(async () => undefined), reject = vi.fn();
  const state = setup(undefined, {onDeactivate, retainPluginStyles: () => true});
  await finishStyles(state.installed); const original = links();
  window.dispatchEvent(new CustomEvent("research-assistant:deactivate-runtime-plugin", {detail: {pluginId, reject, resolve: 3}}));
  await vi.waitFor(() => expect(reject).toHaveBeenCalledWith(expect.objectContaining({code: "plugin_deactivation_invalid"})));
  expect(onDeactivate).not.toHaveBeenCalled(); expect(links()).toEqual(original);
  expect(state.onError).toHaveBeenCalledOnce();
});

it("suppresses late stop callbacks and stylesheet effects after page disposal", async () => {
  const gate = deferred(), onDeactivate = vi.fn(() => gate.promise), resolve = vi.fn(), reject = vi.fn();
  const state = setup(undefined, {onDeactivate, retainPluginStyles: () => false});
  await finishStyles(state.installed);
  const received = broadcasts("research-assistant:runtime-plugin-deactivated");
  window.dispatchEvent(new CustomEvent("research-assistant:deactivate-runtime-plugin", {detail: {pluginId: "note_manager", resolve, reject}}));
  const pending = state.installed.deactivate("note_manager");
  const rejected = expect(pending).rejects.toMatchObject({code: "plugin_loading_disposed"});
  await vi.waitFor(() => expect(onDeactivate).toHaveBeenCalledOnce());
  state.installed.dispose(); gate.resolve(); await rejected; await Promise.resolve();
  window.dispatchEvent(new CustomEvent("research-assistant:deactivate-runtime-plugin", {detail: {pluginId: "note_manager", resolve, reject}}));
  await Promise.resolve();
  expect(resolve).not.toHaveBeenCalled(); expect(reject).not.toHaveBeenCalled();
  expect(received).not.toHaveBeenCalled(); expect(state.onError).not.toHaveBeenCalled();
  expect(onDeactivate).toHaveBeenCalledOnce(); expect(links()).toEqual([]);
  await expect(state.installed.deactivate("note_manager")).rejects.toMatchObject({code: "plugin_loading_disposed"});
});

it("serializes activation, safe stop and a post-stop load without reusing the pre-stop scan", async () => {
  const active = new Set<string>(), activation = deferred(), stopping = deferred(), order: string[] = [];
  const onDeactivate = vi.fn(async () => { order.push("stop"); await stopping.promise; active.clear(); });
  const state = setup(undefined, {onDeactivate, retainPluginStyles: entry => active.has(entry)});
  state.onLoaded.mockImplementation(async () => {
    order.push("activate"); active.add(plugin().entryUrl);
    if (state.onLoaded.mock.calls.length === 1) await activation.promise;
  });
  const first = state.installed.load(); await Promise.resolve();
  const original = links()[0]; original?.dispatchEvent(new Event("load"));
  await vi.waitFor(() => expect(state.onLoaded).toHaveBeenCalledOnce());
  const stop = state.installed.deactivate("note_manager"), after = state.installed.load();
  expect(after).not.toBe(first); expect(state.installed.load()).toBe(after);
  expect(onDeactivate).not.toHaveBeenCalled();
  activation.resolve(); await first;
  await vi.waitFor(() => expect(onDeactivate).toHaveBeenCalledOnce());
  expect(state.onLoaded).toHaveBeenCalledOnce(); expect(links()).toEqual([original]);
  stopping.resolve(); await stop;
  await vi.waitFor(() => expect(links()).toHaveLength(1));
  expect(original?.isConnected).toBe(false);
  links()[0]?.dispatchEvent(new Event("load")); await after;
  expect(order).toEqual(["activate", "stop", "activate"]);
  expect(state.loader.load).toHaveBeenCalledTimes(2); expect(links()).toHaveLength(1);
});

it("can stop a partially activated owner after a failed scan and retains a shared href's live second owner", async () => {
  const first = plugin("first"), second = {...plugin("second", 2), styles: plugin("first").styles};
  const active = new Set<string>(), cause = Error("second owner activation failed");
  const state = setup(result(first, second), {retainPluginStyles: entry => active.has(entry),
    onDeactivate: async () => { active.delete(second.entryUrl); }});
  state.onLoaded.mockImplementationOnce(() => { active.add(second.entryUrl); throw cause; });
  const pending = state.installed.load(), rejected = expect(pending).rejects.toBe(cause);
  await Promise.resolve(); links()[0]?.dispatchEvent(new Event("load")); await rejected;
  expect(links()).toHaveLength(1);
  await state.installed.deactivate("second"); expect(links()).toEqual([]);
});

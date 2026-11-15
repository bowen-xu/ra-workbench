// @vitest-environment happy-dom
import {act, useEffect, useState} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {decodePluginManifest, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {PluginActivationContext, PluginViewContribution, PluginViewProps} from "@research-assistant/workbench-sdk/activation";
import type {RegisteredPluginView} from "../../../core/runtime/plugins/viewRegistry";
import {createRuntimePluginWorkbench} from "./runtimePluginWorkbench";
import {RuntimePluginViewBoundary, type RuntimePluginViewFailure} from "./RuntimePluginViewBoundary";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const roots: Root[] = [];
afterEach(async () => {
  for (const root of roots.splice(0)) await act(async () => root.unmount());
  document.body.replaceChildren();
  localStorage.clear();
  vi.restoreAllMocks();
});

async function registered(render: PluginViewContribution["render"]) {
  const workbench = createRuntimePluginWorkbench({storage: localStorage});
  const manifest = decodePluginManifest({formatVersion: 1, id: "fixture", version: "1.0.0",
    apiVersion: 1, entry: "entry.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME,
    dependencies: {required: [], optional: []}, provides: [], consumes: {required: [], optional: []}});
  const activate = vi.fn((context: PluginActivationContext) => { context.registerView({id: "fixture-view", title: "Fixture view", render}); });
  await workbench.activate({plugins: [{manifest, module: {activate}, entryUrl: "/fixture/entry.js", styles: []}],
    bindings: [], unavailableOptional: []});
  const view = workbench.views.getSnapshot()[0];
  if (!view) throw Error("Fixture registration missing");
  return {workbench, view, activate};
}

function deferred() {
  let resolve: () => void = () => { throw Error("Deferred executor missing"); };
  let reject: (reason: unknown) => void = () => { throw Error("Deferred executor missing"); };
  const promise = new Promise<void>((accept, refuse) => { resolve = accept; reject = refuse; });
  return {promise, resolve, reject};
}

function setup(onRecover: (registration: RegisteredPluginView) => void | Promise<void> = () => undefined) {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host, {onCaughtError: vi.fn()}); roots.push(root);
  const onFailure = vi.fn<(registration: RegisteredPluginView, failure: RuntimePluginViewFailure) => void>();
  const save = vi.fn();
  const render = async (registration: RegisteredPluginView, viewProps: PluginViewProps = {active: true}) => {
    await act(async () => root.render(<>
      <section aria-label="Core sibling"><textarea defaultValue="retained Core draft"/>
        <button type="button" onClick={save}>保存 Core 笔记</button></section>
      <RuntimePluginViewBoundary registration={registration} viewProps={viewProps}
        onFailure={onFailure} onRecover={onRecover}/>
    </>));
  };
  const retry = (): HTMLButtonElement => {
    const button = host.querySelector<HTMLButtonElement>('[role="alert"] button');
    if (!button) throw Error("Recovery button missing");
    return button;
  };
  return {host, root, render, retry, onFailure, save};
}

it("contains a contribution render failure while preserving interactive Core sibling DOM", async () => {
  const issue = Object.assign(Error("timeout at /private/document with secret=fixture-body"),
    {code: "FIXTURE_RENDER_TIMEOUT", requestId: "request-safe", turnId: "turn-safe", operationId: "operation-safe", taskId: "task-safe", status: 503});
  const {view} = await registered(() => { throw issue; });
  const f = setup();
  await f.render(view);
  const editor = f.host.querySelector("textarea");
  if (!editor) throw Error("Core editor missing");
  editor.value = "Core input survives";
  expect(f.host.querySelector('[role="alert"]')?.textContent).toContain("plugin_view_render_failed");
  expect(f.host.textContent).toContain("pluginId=fixture");
  expect(f.host.textContent).toContain("viewId=fixture-view");
  expect(f.host.textContent).toContain("cause=timeout");
  expect(f.host.textContent).toContain("FIXTURE_RENDER_TIMEOUT");
  expect(f.host.textContent).toContain("requestId=request-safe");
  expect(f.host.textContent).toContain("turnId=turn-safe");
  expect(f.host.textContent).toContain("operationId=operation-safe");
  expect(f.host.textContent).toContain("taskId=task-safe");
  expect(f.host.textContent).toContain("HTTP=503");
  expect(f.host.textContent).not.toContain("fixture-body");
  expect(f.host.textContent).not.toContain("/private/document");
  expect(f.onFailure).toHaveBeenCalledExactlyOnceWith(view, {
    code: "plugin_view_render_failed", pluginId: "fixture", viewId: "fixture-view",
    cause: "timeout", sourceCode: "FIXTURE_RENDER_TIMEOUT", requestId: "request-safe", turnId: "turn-safe",
    operationId: "operation-safe", taskId: "task-safe", httpStatus: 503,
  });
  await f.render(view, {active: false});
  expect(f.host.querySelector("textarea")).toBe(editor);
  expect(editor.value).toBe("Core input survives");
  await act(async () => f.host.querySelector<HTMLButtonElement>('[aria-label="Core sibling"] button')?.click());
  expect(f.save).toHaveBeenCalledOnce();
});

it("keeps a healthy view mounted for ordinary props changes of the same registration", async () => {
  const mounted = vi.fn(), unmounted = vi.fn();
  const {view} = await registered(({active}) => {
    useEffect(() => { mounted(); return unmounted; }, []);
    return <output>{active ? "active" : "retained"}</output>;
  });
  const f = setup(); await f.render(view);
  const original = f.host.querySelector("output");
  await f.render(view, {active: false});
  expect(f.host.querySelector("output")).toBe(original);
  expect(original?.textContent).toBe("retained");
  expect(mounted).toHaveBeenCalledOnce(); expect(unmounted).not.toHaveBeenCalled();
});

it("explicitly reopens only the failed view without activating its package again", async () => {
  let broken = true;
  const {view, activate, workbench} = await registered(() => {
    if (broken) throw Error("render failed");
    return <output>Recovered existing registration</output>;
  });
  const onRecover = vi.fn(async (registration: RegisteredPluginView) => {
    expect(workbench.views.getSnapshot()).toContain(registration); broken = false;
  });
  const f = setup(onRecover); await f.render(view);
  const editor = f.host.querySelector("textarea");
  await act(async () => f.retry().click());
  expect(onRecover).toHaveBeenCalledExactlyOnceWith(view);
  expect(f.host.querySelector('[role="alert"]')).toBeNull();
  expect(f.host.querySelector("output")?.textContent).toBe("Recovered existing registration");
  expect(f.host.querySelector("textarea")).toBe(editor);
  expect(activate).toHaveBeenCalledOnce();
  expect(workbench.views.getSnapshot()).toEqual([view]);
});

it("resets for a new registration object even when plugin ID, view ID and key are reused", async () => {
  const {view} = await registered(() => { throw Error("old registration failed"); });
  const f = setup(); await f.render(view);
  const replacement = Object.freeze({...view, render: () => <output>New registration</output>});
  const editor = f.host.querySelector("textarea");
  await f.render(replacement);
  expect(f.host.querySelector('[role="alert"]')).toBeNull();
  expect(f.host.querySelector("output")?.textContent).toBe("New registration");
  expect(f.host.querySelector("textarea")).toBe(editor);
  expect(f.onFailure).toHaveBeenCalledTimes(1);
});

it("releases only the old view's hooks when a healthy registration is replaced", async () => {
  const oldMounted = vi.fn(), oldUnmounted = vi.fn(), newMounted = vi.fn();
  const {view} = await registered(() => {
    useEffect(() => { oldMounted(); return oldUnmounted; }, []);
    return <output>Original view</output>;
  });
  const f = setup(); await f.render(view);
  const oldOutput = f.host.querySelector("output"), editor = f.host.querySelector("textarea");
  const replacement = Object.freeze({...view, render: () => {
    const [text] = useState("New view owns its hooks");
    useEffect(newMounted, []);
    return <output>{text}</output>;
  }});
  await f.render(replacement);
  expect(oldOutput?.isConnected).toBe(false);
  expect(f.host.querySelector("output")?.textContent).toBe("New view owns its hooks");
  expect(f.host.querySelector("textarea")).toBe(editor);
  expect(oldMounted).toHaveBeenCalledOnce(); expect(oldUnmounted).toHaveBeenCalledOnce();
  expect(newMounted).toHaveBeenCalledOnce(); expect(f.onFailure).not.toHaveBeenCalled();
});

it.each(["resolve", "reject"] as const)("ignores an old recovery's late %s after registration replacement", async result => {
  const pending = deferred(), onRecover = vi.fn(() => pending.promise);
  const {view} = await registered(() => { throw Error("old registration failed"); });
  const f = setup(onRecover); await f.render(view);
  await act(async () => { f.retry().click(); f.retry().click(); });
  expect(onRecover).toHaveBeenCalledExactlyOnceWith(view);
  expect(f.retry().disabled).toBe(true);
  let broken = false;
  const replacement = Object.freeze({...view, render: () => {
    if (broken) throw Error("new registration failure");
    return <output>Replacement view</output>;
  }});
  await f.render(replacement);
  broken = true; await f.render(replacement);
  expect(f.onFailure).toHaveBeenLastCalledWith(replacement, expect.objectContaining({code: "plugin_view_render_failed"}));
  await act(async () => {
    if (result === "resolve") pending.resolve(); else pending.reject(Error("old recovery rejected"));
    await pending.promise.catch(() => undefined);
  });
  expect(f.host.querySelector('[role="alert"]')).not.toBeNull();
  expect(f.host.textContent).not.toContain("plugin_view_recovery_failed");
  expect(onRecover).toHaveBeenCalledTimes(1);
  expect(f.onFailure).toHaveBeenCalledTimes(2);
});

it("keeps a rejected reopen visible and permits an explicit retry", async () => {
  const {view} = await registered(() => { throw Error("render failed"); });
  const onRecover = vi.fn(async () => { throw Object.assign(Error("permission denied /private/content"),
    {code: "VIEW_OPEN_BLOCKED", requestId: "reopen-request"}); });
  const f = setup(onRecover); await f.render(view);
  await act(async () => f.retry().click());
  expect(f.host.textContent).toContain("plugin_view_recovery_failed");
  expect(f.host.textContent).toContain("VIEW_OPEN_BLOCKED");
  expect(f.host.textContent).toContain("requestId=reopen-request");
  expect(f.host.textContent).not.toContain("/private/content");
  expect(f.retry().disabled).toBe(false);
  await act(async () => f.retry().click());
  expect(onRecover).toHaveBeenCalledTimes(2);
});

it("keeps the fallback safe even when a thrown object's diagnostic getter throws", async () => {
  const reason = {get message(): never { throw Error("private getter failure"); }};
  const {view} = await registered(() => { throw reason; });
  const f = setup(); await f.render(view);
  expect(f.host.textContent).toContain("cause=unclassified");
  expect(f.host.textContent).not.toContain("private getter");
  expect(f.onFailure).toHaveBeenCalledExactlyOnceWith(view, expect.objectContaining({cause: "unclassified"}));
});

it("uses finite role content with the original registration and accepts an empty role without falling back to its workspace", async () => {
  const workspace = vi.fn(() => "Workspace must stay unopened");
  const {view} = await registered(workspace);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host, {onCaughtError: vi.fn()}); roots.push(root);
  const onFailure = vi.fn();
  await act(async () => root.render(<RuntimePluginViewBoundary registration={view} viewProps={{active: false}}
    renderContent={() => null} onFailure={onFailure} onRecover={() => undefined}/>));
  expect(workspace).not.toHaveBeenCalled();
  await act(async () => root.render(<RuntimePluginViewBoundary registration={view} viewProps={{active: false}}
    renderContent={() => {throw Object.assign(new Error("Role failed"), {code: "ROLE_UNAVAILABLE"});}}
    onFailure={onFailure} onRecover={() => undefined}/>));
  expect(host.textContent).toContain("ROLE_UNAVAILABLE");
  expect(onFailure).toHaveBeenCalledWith(view, expect.objectContaining({pluginId: view.pluginId, viewId: view.id}));
  expect(workspace).not.toHaveBeenCalled();
});

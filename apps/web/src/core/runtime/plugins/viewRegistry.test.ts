import {afterEach, expect, it, vi} from "vitest";
import type {PluginViewContribution} from "@research-assistant/workbench-sdk/activation";
import {createPluginViewRegistry, PluginViewRegistryError} from "./viewRegistry";
import type {RegisteredPluginView} from "./viewRegistry";

afterEach(() => { vi.useRealTimers(); });

function view(id = "notes", title = "笔记"): PluginViewContribution {
  return {id, title, render: () => title};
}

function deferred<T>() {
  let resolve: (value: T) => void = () => { throw new Error("Promise executor did not initialize"); };
  let reject: (reason: unknown) => void = () => { throw new Error("Promise executor did not initialize"); };
  const promise = new Promise<T>((accept, refuse) => { resolve = accept; reject = refuse; });
  return {promise, resolve, reject};
}

function setup() {
  const open = vi.fn(async (_view: RegisteredPluginView): Promise<void> => undefined);
  const prepareClose = vi.fn(async (_view: RegisteredPluginView): Promise<boolean> => true);
  return {open, prepareClose, registry: createPluginViewRegistry({open, prepareClose})};
}

it("publishes frozen stable snapshots only when registrations change", async () => {
  const {registry, open} = setup(), changed = vi.fn(() => registry.getSnapshot());
  const initial = registry.getSnapshot();
  expect(registry.getSnapshot()).toBe(initial);
  const unsubscribe = registry.subscribe(changed);
  const contribution = {...view()};
  const registration = registry.register("note_manager", contribution);
  const snapshot = registry.getSnapshot();
  expect(changed).toHaveReturnedWith(snapshot);
  expect(snapshot).not.toBe(initial);
  expect(Object.isFrozen(snapshot)).toBe(true);
  expect(Object.isFrozen(snapshot[0])).toBe(true);
  contribution.title = "Changed caller-owned object";
  expect(snapshot[0]?.title).toBe("笔记");
  await registration.open();
  expect(open).toHaveBeenCalledExactlyOnceWith(snapshot[0], expect.any(AbortSignal));
  expect(registry.getSnapshot()).toBe(snapshot);
  expect(changed).toHaveBeenCalledTimes(1);
  unsubscribe(); unsubscribe();
  await registration.dispose();
  expect(registry.getSnapshot()).toEqual([]);
  expect(changed).toHaveBeenCalledTimes(1);
});

it("keeps independent subscriptions and makes each cleanup idempotent", () => {
  const {registry} = setup(), changed = vi.fn();
  const first = registry.subscribe(changed), second = registry.subscribe(changed);
  first(); first();
  registry.register("note_manager", view());
  expect(changed).toHaveBeenCalledTimes(1);
  second(); second();
  registry.register("note_manager", view("notes-fixture", "Fixture"));
  expect(changed).toHaveBeenCalledTimes(1);
});

it("retains optional frozen display priority without reordering registrations", () => {
  const {registry} = setup();
  const contribution = {...view("writing"), priority: 100};
  registry.register("writing_projects", contribution);
  registry.register("note_manager", {...view(), priority: 200});
  contribution.priority = 900;
  expect(registry.getSnapshot().map(({id, priority}) => ({id, priority}))).toEqual([
    {id: "writing", priority: 100}, {id: "notes", priority: 200},
  ]);
});

it("uses owner-local view IDs and accepts the second package fixture", () => {
  const {registry} = setup();
  registry.register("note_manager", view());
  registry.register("note_manager", view("notes-fixture", "Fixture"));
  registry.register("other_plugin", view());
  expect(registry.getSnapshot().map(({key, pluginId, id}) => ({key, pluginId, id}))).toEqual([
    {key: "plugin:note_manager/notes", pluginId: "note_manager", id: "notes"},
    {key: "plugin:note_manager/notes-fixture", pluginId: "note_manager", id: "notes-fixture"},
    {key: "plugin:other_plugin/notes", pluginId: "other_plugin", id: "notes"},
  ]);
});

it("rejects duplicate owner/view registration without publishing or opening twice", () => {
  const {registry, open} = setup(), changed = vi.fn();
  registry.subscribe(changed);
  registry.register("note_manager", view());
  const snapshot = registry.getSnapshot();
  expect(() => registry.register("note_manager", view())).toThrow(PluginViewRegistryError);
  expect(() => registry.register("note_manager", view())).toThrow("plugin_view_duplicate");
  expect(registry.getSnapshot()).toBe(snapshot);
  expect(changed).toHaveBeenCalledTimes(1);
  expect(open).not.toHaveBeenCalled();
});

it("keeps writing roles with their exact view through refused preparation and retires them only on commit", async () => {
  const {registry, prepareClose} = setup();
  const Runtime = () => null, Session = () => null;
  const writing = {Runtime, Session};
  const registration = registry.register("writing_projects", {...view("writing"), writing});
  const snapshot = registry.getSnapshot(), original = snapshot[0]!;
  expect(original.writing).not.toBe(writing);
  expect(Object.isFrozen(original.writing)).toBe(true);
  writing.Runtime = () => null;
  expect(original.writing).toEqual({Runtime, Session});

  prepareClose.mockResolvedValueOnce(false);
  const refused = registry.beginOwnerClose("writing_projects");
  await expect(refused.prepare()).rejects.toMatchObject({code: "plugin_view_close_blocked"});
  refused.cancel();
  expect(registry.getSnapshot()).toBe(snapshot);
  expect(original.writing?.Runtime).toBe(Runtime);

  const close = registry.beginOwnerClose("writing_projects");
  await close.prepare();
  await registration.dispose();
  expect(registry.getSnapshot()).toBe(snapshot);
  close.commit();
  expect(registry.getSnapshot()).toEqual([]);

  registry.register("writing_projects", {...view("writing"), writing});
  const replacement = registry.getSnapshot()[0]!;
  expect(replacement.key).toBe(original.key);
  expect(replacement).not.toBe(original);
  expect(replacement.writing?.Runtime).toBe(writing.Runtime);
  await registration.dispose();
  expect(registry.getSnapshot()[0]).toBe(replacement);
});

it("coalesces only concurrent open calls and retries a failed open", async () => {
  const {registry, open} = setup(), pending = deferred<void>();
  open.mockImplementationOnce(() => pending.promise);
  const registration = registry.register("note_manager", view());
  const first = registration.open(), concurrent = registration.open();
  expect(concurrent).toBe(first);
  await Promise.resolve();
  expect(open).toHaveBeenCalledTimes(1);
  const reason = new Error("Navigation preparation failed");
  const rejected = expect(first).rejects.toBe(reason);
  pending.reject(reason);
  await rejected;
  await registration.open();
  expect(open).toHaveBeenCalledTimes(2);
});

it("keeps the view mounted until one shared close preparation succeeds", async () => {
  const {registry, prepareClose} = setup(), pending = deferred<boolean>(), changed = vi.fn();
  prepareClose.mockImplementationOnce(() => pending.promise);
  const registration = registry.register("note_manager", view());
  const snapshot = registry.getSnapshot();
  registry.subscribe(changed);
  const first = registration.dispose(), concurrent = registration.dispose();
  expect(concurrent).toBe(first);
  await Promise.resolve();
  expect(prepareClose).toHaveBeenCalledExactlyOnceWith(snapshot[0], expect.any(AbortSignal));
  expect(registry.getSnapshot()).toBe(snapshot);
  expect(changed).not.toHaveBeenCalled();
  await expect(registration.open()).rejects.toMatchObject({code: "plugin_view_closing"});
  expect(() => registry.register("note_manager", view())).toThrow("plugin_view_duplicate");
  pending.resolve(true);
  await first;
  expect(registry.getSnapshot()).toEqual([]);
  expect(changed).toHaveBeenCalledTimes(1);
  expect(registration.dispose()).toBe(first);
  await registration.dispose();
  expect(prepareClose).toHaveBeenCalledTimes(1);
});

it("retains a blocked view and permits open and close retries", async () => {
  const {registry, prepareClose, open} = setup();
  prepareClose.mockResolvedValueOnce(false);
  const registration = registry.register("note_manager", view());
  const snapshot = registry.getSnapshot();
  await expect(registration.dispose()).rejects.toMatchObject({
    code: "plugin_view_close_blocked", pluginId: "note_manager", viewId: "notes",
  });
  expect(registry.getSnapshot()).toBe(snapshot);
  await registration.open();
  expect(open).toHaveBeenCalledTimes(1);
  await registration.dispose();
  expect(prepareClose).toHaveBeenCalledTimes(2);
  expect(registry.getSnapshot()).toEqual([]);
});

it("preserves a close failure and can retry without losing the registration", async () => {
  const {registry, prepareClose} = setup(), reason = new Error("Original Core save failure");
  prepareClose.mockRejectedValueOnce(reason);
  const registration = registry.register("note_manager", view());
  const snapshot = registry.getSnapshot();
  await expect(registration.dispose()).rejects.toBe(reason);
  expect(registry.getSnapshot()).toBe(snapshot);
  await registration.dispose();
  expect(registry.getSnapshot()).toEqual([]);
});

it.each(["resolve", "reject"] as const)("drains an in-flight open that will %s before removing the view", async completion => {
  const {registry, open, prepareClose} = setup(), pending = deferred<void>();
  open.mockImplementationOnce(() => pending.promise);
  const registration = registry.register("note_manager", view());
  const snapshot = registry.getSnapshot();
  const opening = registration.open();
  const openResult = opening.then(() => "opened", () => "failed");
  const closing = registration.dispose();
  await Promise.resolve();
  expect(open).toHaveBeenCalledTimes(1);
  expect(prepareClose).not.toHaveBeenCalled();
  expect(registry.getSnapshot()).toBe(snapshot);
  if (completion === "resolve") pending.resolve(); else pending.reject(new Error("Open failed"));
  expect(await openResult).toBe(completion === "resolve" ? "opened" : "failed");
  await closing;
  expect(prepareClose).toHaveBeenCalledTimes(1);
  expect(registry.getSnapshot()).toEqual([]);
});

it("a revoked handle cannot open or remove the replacement registration", async () => {
  const {registry, open} = setup();
  const previous = registry.register("note_manager", view());
  await previous.dispose();
  const replacement = registry.register("note_manager", view());
  const snapshot = registry.getSnapshot();
  await expect(previous.open()).rejects.toMatchObject({code: "plugin_view_disposed"});
  await previous.dispose();
  expect(registry.getSnapshot()).toBe(snapshot);
  expect(open).not.toHaveBeenCalled();
  await replacement.open();
  expect(open).toHaveBeenCalledExactlyOnceWith(snapshot[0], expect.any(AbortSignal));
});

it("prepares every owner view before one atomic removal and does not prepare handles twice", async () => {
  const {registry, prepareClose} = setup(), gate = deferred<boolean>();
  const first = registry.register("note_manager", view());
  const second = registry.register("note_manager", view("notes-fixture"));
  registry.register("unrelated", view("other"));
  const original = registry.getSnapshot();
  prepareClose.mockResolvedValueOnce(true).mockImplementationOnce(() => gate.promise);
  const close = registry.beginOwnerClose("note_manager");
  const pending = close.prepare();
  await vi.waitFor(() => expect(prepareClose).toHaveBeenCalledTimes(2));
  expect(registry.getSnapshot()).toBe(original);
  await expect(first.open()).rejects.toMatchObject({code: "plugin_view_closing"});
  expect(() => registry.register("note_manager", view("late"))).toThrow("plugin_view_closing");
  gate.resolve(true); await pending;
  await first.dispose(); await second.dispose();
  expect(prepareClose).toHaveBeenCalledTimes(2);
  expect(registry.getSnapshot()).toBe(original);
  close.commit(); close.commit();
  expect(registry.getSnapshot()).toEqual([original[2]]);
  await expect(first.open()).rejects.toMatchObject({code: "plugin_view_disposed"});
});

it("retains all projections when a later owner view refuses close", async () => {
  const {registry, prepareClose} = setup();
  const first = registry.register("note_manager", view());
  registry.register("note_manager", view("notes-fixture"));
  const original = registry.getSnapshot();
  prepareClose.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
  const close = registry.beginOwnerClose("note_manager");
  await expect(close.prepare()).rejects.toMatchObject({code: "plugin_view_close_blocked", viewId: "notes-fixture"});
  expect(registry.getSnapshot()).toBe(original);
  close.cancel(); close.cancel();
  await first.open();
  expect(registry.getSnapshot()).toBe(original);
  expect(() => close.commit()).toThrow("plugin_view_close_blocked");
});

it("cancels an owner attempt without a late preparation removing a replacement", async () => {
  const {registry, prepareClose} = setup(), gate = deferred<boolean>();
  const previous = registry.register("note_manager", view());
  const original = registry.getSnapshot();
  prepareClose.mockImplementationOnce(() => gate.promise);
  const close = registry.beginOwnerClose("note_manager");
  const pending = close.prepare();
  const failure = expect(pending).rejects.toMatchObject({code: "plugin_view_closing"});
  close.cancel(); await failure;
  expect(registry.getSnapshot()).toBe(original);
  await previous.dispose();
  const replacement = registry.register("note_manager", view());
  const current = registry.getSnapshot();
  gate.resolve(true); await Promise.resolve();
  expect(registry.getSnapshot()).toBe(current);
  expect(() => close.commit()).toThrow("plugin_view_close_blocked");
  await replacement.open();
});

it.each(["open", "prepare"] as const)("bounds a lost %s without a late close removing the view", async phase => {
  vi.useFakeTimers();
  const gate = deferred<boolean>();
  const open = vi.fn(async (_view: RegisteredPluginView, _signal?: AbortSignal) => {
    if (phase === "open") await gate.promise;
  });
  const prepareClose = vi.fn(async (_view: RegisteredPluginView, _signal?: AbortSignal) => phase === "prepare" ? gate.promise : true);
  const registry = createPluginViewRegistry({open, prepareClose}, {closeTimeoutMs: 20});
  const registration = registry.register("note_manager", view());
  const original = registry.getSnapshot();
  const opening = phase === "open" ? registration.open().catch(reason => reason as unknown) : Promise.resolve();
  const closing = registration.dispose();
  const failed = expect(closing).rejects.toMatchObject({code: "plugin_view_close_timeout"});
  await vi.advanceTimersByTimeAsync(20); await failed; await opening;
  expect(registry.getSnapshot()).toBe(original);
  const signal = (phase === "open" ? open : prepareClose).mock.calls[0]?.[1];
  expect(signal?.aborted).toBe(true);
  gate.resolve(true); await Promise.resolve(); await Promise.resolve();
  expect(registry.getSnapshot()).toBe(original);
  if (phase === "open") expect(prepareClose).not.toHaveBeenCalled();
});

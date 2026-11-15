import {expect, it, vi} from "vitest";
import {createPluginViewRegistry} from "../../core/runtime/plugins/viewRegistry";
import {ReaderCloseCoordinator} from "../../core/workspace/DocumentCloseCoordinator";
import {captureViewNavigationGuard} from "./viewNavigationGuard";

function deferred<T>() {
  let resolve: (value: T) => void = () => { throw new Error("Promise executor did not initialize"); };
  const promise = new Promise<T>(accept => { resolve = accept; });
  return {promise, resolve};
}

function setup() {
  const registry = createPluginViewRegistry({open: vi.fn(async () => undefined), prepareClose: vi.fn(async () => true)});
  const contribution = {id: "notes-fixture", title: "Fixture", render: () => "Fixture"};
  const registered = registry.register("note_manager", contribution);
  const save = deferred<boolean>(), coordinator = new ReaderCloseCoordinator();
  coordinator.register("core-note", {isActive: () => true, prepare: () => save.promise, commit: vi.fn()});
  const targetIsCurrent = captureViewNavigationGuard({view: contribution.id, presentation: "workspace", registry});
  const navigate = vi.fn(), cancelled = vi.fn();
  // Use the original coordinator and registry, injecting only the pending save
  // effect, then call chooseView's production guard at its commit point.
  const pending = coordinator.prepareAll().then(saved => {
    if (saved && targetIsCurrent()) navigate(contribution.id); else cancelled();
  });
  return {registry, contribution, registered, save, navigate, cancelled, pending};
}

it("cancels a late navigation when its target is withdrawn during Core preservation", async () => {
  const state = setup();
  expect(state.navigate).not.toHaveBeenCalled();
  await state.registered.dispose();
  state.save.resolve(true);
  await state.pending;
  expect(state.navigate).not.toHaveBeenCalled();
  expect(state.cancelled).toHaveBeenCalledTimes(1);
});

it("does not let a replacement with the same owner and ID accept the previous navigation", async () => {
  const state = setup();
  await state.registered.dispose();
  state.registry.register("note_manager", state.contribution);
  state.save.resolve(true);
  await state.pending;
  expect(state.navigate).not.toHaveBeenCalled();
  expect(state.cancelled).toHaveBeenCalledTimes(1);
});

it("commits after Core preservation when the original registration remains present", async () => {
  const state = setup();
  state.save.resolve(true);
  await state.pending;
  expect(state.navigate).toHaveBeenCalledExactlyOnceWith("notes-fixture");
  expect(state.cancelled).not.toHaveBeenCalled();
});

it("rejects a stale public ID that was already absent when navigation began", () => {
  expect(captureViewNavigationGuard({view: "notes-fixture", presentation: "workspace"})()).toBe(false);
});

it("requires a registration for management routes and keeps Core routes available", () => {
  for (const view of ["settings", "notes"]) {
    expect(captureViewNavigationGuard({view, presentation: "workspace"})()).toBe(true);
  }
  for (const view of ["library", "writing"]) {
    expect(captureViewNavigationGuard({view, presentation: "workspace"})()).toBe(false);
  }
});

it("keeps a Core single-note document route valid after its manager is withdrawn", async () => {
  const registry = createPluginViewRegistry({open: vi.fn(async () => undefined), prepareClose: vi.fn(async () => true)});
  const manager = registry.register("note_manager", {id: "notes", title: "笔记", render: () => "笔记"});
  const targetIsCurrent = captureViewNavigationGuard({view: "notes", presentation: "document", registry});
  await manager.dispose();
  expect(targetIsCurrent()).toBe(true);
});

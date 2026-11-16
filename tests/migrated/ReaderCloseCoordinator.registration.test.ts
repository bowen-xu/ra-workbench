import { expect, it, vi } from "vitest";
import { ReaderCloseCoordinator } from "../../apps/web/src/core/workspace/DocumentCloseCoordinator";

it("prepares each captured owner once when a render replaces its registration", async () => {
  const coordinator = new ReaderCloseCoordinator();
  let unregister = () => {};
  const prepare = vi.fn(async () => {
    // A save publishes a new snapshot. React cleans up the previous boundary
    // and installs the same workspace owner while prepareAll is awaiting it.
    if (prepare.mock.calls.length < 4) {
      unregister();
      register();
    }
    return true;
  });
  const register = () => {
    unregister = coordinator.register("notes-workspace", {
      isActive: () => false,
      prepare,
      commit: () => {},
    });
  };
  register();
  expect(await coordinator.prepareAll()).toBe(true);
  expect(prepare).toHaveBeenCalledOnce();
  unregister();
});

it("checks all original owners and defers newly registered owners to the next operation", async () => {
  const coordinator = new ReaderCloseCoordinator();
  const added = vi.fn(async () => true);
  const second = vi.fn(async () => true);
  const first = vi.fn(async () => {
    coordinator.register("new-session", { isActive: () => false, prepare: added, commit: () => {} });
    return true;
  });
  coordinator.register("first", { isActive: () => false, prepare: first, commit: () => {} });
  coordinator.register("second", { isActive: () => false, prepare: second, commit: () => {} });
  expect(await coordinator.prepareAll()).toBe(true);
  expect(first).toHaveBeenCalledOnce();
  expect(second).toHaveBeenCalledOnce();
  expect(added).not.toHaveBeenCalled();
  expect(await coordinator.prepareAll()).toBe(true);
  expect(first).toHaveBeenCalledTimes(2);
  expect(second).toHaveBeenCalledTimes(2);
  expect(added).toHaveBeenCalledOnce();
});

it("still stops preparation at the first failed owner without committing any close", async () => {
  const coordinator = new ReaderCloseCoordinator();
  const commit = vi.fn();
  const failed = vi.fn(async () => false);
  const later = vi.fn(async () => true);
  coordinator.register("failed", { isActive: () => false, prepare: failed, commit });
  coordinator.register("later", { isActive: () => false, prepare: later, commit });
  expect(await coordinator.prepareAll()).toBe(false);
  expect(failed).toHaveBeenCalledOnce();
  expect(later).not.toHaveBeenCalled();
  expect(commit).not.toHaveBeenCalled();
});

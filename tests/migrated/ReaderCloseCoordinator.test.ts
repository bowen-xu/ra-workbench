import { describe, expect, it, vi } from "vitest";

import { ReaderCloseCoordinator } from "../../apps/web/src/core/workspace/DocumentCloseCoordinator";

describe("ReaderCloseCoordinator", () => {
  it("prepares durable recovery before committing a tab close and blocks only when preparation fails", async () => {
    const coordinator = new ReaderCloseCoordinator();
    const blockedCommit = vi.fn();
    coordinator.register("blocked", { isActive: () => true, prepare: vi.fn().mockResolvedValue(false), commit: blockedCommit });
    expect(await coordinator.closeActive()).toBe(false);
    expect(blockedCommit).not.toHaveBeenCalled();

    const commit = vi.fn();
    coordinator.register("ready", { isActive: () => true, prepare: vi.fn().mockResolvedValue(true), commit });
    expect(await coordinator.close("ready")).toBe(true);
    expect(commit).toHaveBeenCalledOnce();
  });

  it("prepares every mounted session without committing before app quit", async () => {
    const coordinator = new ReaderCloseCoordinator();
    const first = vi.fn().mockResolvedValue(true);
    const second = vi.fn().mockResolvedValue(true);
    const commit = vi.fn();
    coordinator.register("first", { isActive: () => false, prepare: first, commit });
    coordinator.register("second", { isActive: () => true, prepare: second, commit });
    expect(await coordinator.prepareAll()).toBe(true);
    expect(first).toHaveBeenCalledOnce();
    expect(second).toHaveBeenCalledOnce();
    expect(commit).not.toHaveBeenCalled();
  });

  it("coalesces concurrent close requests into one prepare and commit", async () => {
    const coordinator = new ReaderCloseCoordinator();
    let release: ((ready: boolean) => void) | undefined;
    const prepare = vi.fn(() => new Promise<boolean>((resolve) => { release = resolve; }));
    const commit = vi.fn();
    coordinator.register("reader", { isActive: () => true, prepare, commit });

    const first = coordinator.close("reader");
    const second = coordinator.close("reader");
    release?.(true);

    await expect(Promise.all([first, second])).resolves.toEqual([true, true]);
    expect(prepare).toHaveBeenCalledOnce();
    expect(commit).toHaveBeenCalledOnce();
  });
});

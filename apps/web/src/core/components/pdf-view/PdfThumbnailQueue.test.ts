// @vitest-environment happy-dom

import { describe, expect, it, vi } from "vitest";

import { PdfThumbnailQueue, type PdfThumbnailJob } from "./PdfThumbnailQueue";

function deferred(): { readonly promise: Promise<void>; readonly resolve: () => void } {
  let resolve = (): void => undefined;
  const promise = new Promise<void>((settle) => { resolve = settle; });
  return { promise, resolve };
}

describe("PdfThumbnailQueue", () => {
  it("renders at most one thumbnail at a time", async () => {
    const first = deferred();
    const second = deferred();
    let active = 0;
    let maximum = 0;
    const render = vi.fn(async (job: PdfThumbnailJob) => {
      active += 1;
      maximum = Math.max(maximum, active);
      await (job.pageNumber === 1 ? first.promise : second.promise);
      active -= 1;
    });
    const queue = new PdfThumbnailQueue(render);
    const firstJob = queue.enqueue({ pageNumber: 1, canvas: document.createElement("canvas") });
    const secondJob = queue.enqueue({ pageNumber: 2, canvas: document.createElement("canvas") });

    await Promise.resolve();
    expect(render).toHaveBeenCalledTimes(1);
    first.resolve();
    await firstJob;
    await Promise.resolve();
    expect(render).toHaveBeenCalledTimes(2);
    second.resolve();
    await secondJob;
    expect(maximum).toBe(1);
  });

  it("skips aborted work and clears pending jobs while paused", async () => {
    const running = deferred();
    const render = vi.fn(async () => running.promise);
    const queue = new PdfThumbnailQueue(render);
    const controller = new AbortController();
    controller.abort();
    await expect(queue.enqueue({ pageNumber: 1, canvas: document.createElement("canvas"), signal: controller.signal })).rejects.toMatchObject({name:"AbortError"});
    expect(render).not.toHaveBeenCalled();

    const first = queue.enqueue({ pageNumber: 2, canvas: document.createElement("canvas") });
    const pending = queue.enqueue({ pageNumber: 3, canvas: document.createElement("canvas") });
    const firstCancelled=expect(first).rejects.toMatchObject({name:"AbortError"});
    const pendingCancelled=expect(pending).rejects.toMatchObject({name:"AbortError"});
    await Promise.resolve();
    queue.pause();
    const idle=vi.fn();void queue.whenIdle().then(idle);await Promise.resolve();expect(idle).not.toHaveBeenCalled();
    await pendingCancelled;
    expect(render).toHaveBeenCalledTimes(1);
    running.resolve();
    await firstCancelled;
    await queue.whenIdle();expect(idle).toHaveBeenCalledOnce();
    queue.resume();
    expect(render).toHaveBeenCalledTimes(1);
  });
});

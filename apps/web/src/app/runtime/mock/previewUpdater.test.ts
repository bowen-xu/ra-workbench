import { afterEach, expect, it, vi } from "vitest";
import { createPreviewUpdater } from "../../../core/runtime/previewUpdater";

afterEach(() => vi.useRealTimers());

it("defaults to an available update, downloads in five seconds, then stays current after installation", async () => {
  vi.useFakeTimers();
  const gateway = createPreviewUpdater(undefined);
  expect((await gateway.check()).phase).toBe("available");
  expect((await gateway.check()).phase).toBe("available");
  const download = gateway.download();
  await vi.advanceTimersByTimeAsync(4999);
  expect((await gateway.status()).phase).toBe("downloading");
  expect((await gateway.status()).downloaded).toBe(99);
  await vi.advanceTimersByTimeAsync(1);
  expect((await download).phase).toBe("downloaded");
  const installation = gateway.install();
  await vi.advanceTimersByTimeAsync(500);
  expect((await installation).phase).toBe("upToDate");
  expect((await gateway.check()).phase).toBe("upToDate");
  expect((await gateway.check()).message).toContain("当前已是最新版本");
  // Reloading the preview starts a fresh demonstration.
  expect((await createPreviewUpdater(undefined).check()).phase).toBe("available");
});

it("holds progress while paused and resumes without restarting the five-second download", async () => {
  vi.useFakeTimers();
  const gateway = createPreviewUpdater(undefined);
  await gateway.check();
  const download = gateway.download();
  await vi.advanceTimersByTimeAsync(2000);
  await gateway.pause(true);
  const paused = await gateway.status();
  await vi.advanceTimersByTimeAsync(10_000);
  expect((await gateway.status()).downloaded).toBe(paused.downloaded);
  expect((await gateway.status()).phase).toBe("paused");
  await gateway.pause(false);
  await vi.advanceTimersByTimeAsync(3000);
  expect((await download).downloaded).toBe(100);
});

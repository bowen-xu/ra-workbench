// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { browserConversationOperationLock } from "../conversationOperationLock";

const original = Object.getOwnPropertyDescriptor(navigator, "locks");
afterEach(() => {
  if (original) Object.defineProperty(navigator, "locks", original);
  else Reflect.deleteProperty(navigator, "locks");
  vi.useRealTimers();
});

it("waits for the existing owner before running the next context operation", async () => {
  let held = false, release: (() => void) | undefined;
  let tail = Promise.resolve();
  Object.defineProperty(navigator, "locks", {configurable: true, value: {request: (_name: string, options: LockOptions, callback: (lock: object) => Promise<unknown>) => {
    expect(options.ifAvailable).toBeUndefined();
    const task = tail.then(async () => { held = true; try { return await callback({}); } finally { held = false; } });
    tail = task.then(() => undefined, () => undefined); return task;
  }}});
  const lock = browserConversationOperationLock("CONTEXT_CONFIGURATION_RECOVERY", {waitMs: 10000});
  const events: string[] = [];
  const first = lock.run("same", async () => { events.push("first"); await new Promise<void>(resolve => {release = resolve;}); });
  await vi.waitFor(() => expect(held).toBe(true));
  const second = lock.run("same", async () => { events.push("second"); });
  await Promise.resolve(); expect(events).toEqual(["first"]);
  release!(); await Promise.all([first, second]);
  expect(events).toEqual(["first", "second"]);
});

it("reports a bounded wait failure without starting the protected operation", async () => {
  vi.useFakeTimers();
  Object.defineProperty(navigator, "locks", {configurable: true, value: {request: (_name: string, options: LockOptions) =>
    new Promise((_resolve, reject) => options.signal!.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError"))))}});
  const operation = vi.fn();
  const pending = browserConversationOperationLock("CONTEXT_CONFIGURATION_RECOVERY", {waitMs: 100}).run("same", operation);
  const result = expect(pending).rejects.toThrow("CONTEXT_CONFIGURATION_RECOVERY_LOCK_BUSY");
  await vi.advanceTimersByTimeAsync(100); await result;
  expect(operation).not.toHaveBeenCalled();
});

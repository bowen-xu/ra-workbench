import {expect, it, vi} from "vitest";
import {invokeDesktopHistoryWait} from "./desktopHistoryWait";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((ok, fail) => {resolve = ok; reject = fail;});
  return {promise, resolve, reject};
}

it("does not create a resource when the history owner has already stopped", async () => {
  const signal = new AbortController(); signal.abort();
  const invoke = vi.fn();
  await expect(invokeDesktopHistoryWait(invoke, {request: {}}, signal.signal)).rejects.toMatchObject({name: "AbortError"});
  expect(invoke).not.toHaveBeenCalled();
});

it("closes a resource returned after its history owner has stopped without starting a wait", async () => {
  const creation = deferred<unknown>();
  const invoke = vi.fn((command: string) => command === "agent_create_history_wait" ? creation.promise : Promise.resolve());
  const signal = new AbortController();
  const result = invokeDesktopHistoryWait(invoke, {request: {}}, signal.signal);
  const rejected = expect(result).rejects.toMatchObject({name: "AbortError"});
  signal.abort(); creation.resolve(17);
  await rejected;
  expect(invoke.mock.calls).toEqual([["agent_create_history_wait", {}], ["agent_cancel_history_wait", {resourceId: 17}]]);
});

it("cancels an in-flight wait once, including when the native response races with cancellation", async () => {
  const started = deferred<void>(); const pending = deferred<unknown>();
  const invoke = vi.fn((command: string) => {
    if (command === "agent_create_history_wait") return Promise.resolve(23);
    if (command === "agent_wait_for_history_cancellable") {started.resolve(); return pending.promise;}
    return Promise.resolve();
  });
  const signal = new AbortController();
  const result = invokeDesktopHistoryWait(invoke, {request: {afterSeq: 7}}, signal.signal);
  const rejected = expect(result).rejects.toMatchObject({name: "AbortError"});
  await started.promise; signal.abort();
  await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("agent_cancel_history_wait", {resourceId: 23}));
  pending.resolve({watermark: 8});
  await rejected;
  expect(invoke.mock.calls.filter(([command]) => command === "agent_cancel_history_wait")).toHaveLength(1);
  expect(invoke).toHaveBeenCalledWith("agent_wait_for_history_cancellable", {request: {afterSeq: 7}, resourceId: 23});
});

it("cleans up completed and failed waits without hiding the original read error", async () => {
  const readFailure = new Error("read failed");
  for (const failed of [false, true]) {
    const invoke = vi.fn((command: string) => {
      if (command === "agent_create_history_wait") return Promise.resolve(3);
      if (command === "agent_wait_for_history_cancellable") return failed ? Promise.reject(readFailure) : Promise.resolve({watermark: 1});
      return Promise.resolve();
    });
    const result = invokeDesktopHistoryWait(invoke, {request: {}}, new AbortController().signal);
    if (failed) await expect(result).rejects.toBe(readFailure);
    else await expect(result).resolves.toEqual({watermark: 1});
    expect(invoke).toHaveBeenCalledWith("agent_cancel_history_wait", {resourceId: 3});
  }
});

it("reports cleanup failure without replacing an existing wait failure", async () => {
  const readFailure = new Error("read failed"), closeFailure = new Error("cleanup failed");
  for (const readFails of [false, true]) {
    const invoke = vi.fn((command: string) => {
      if (command === "agent_create_history_wait") return Promise.resolve(3);
      if (command === "agent_wait_for_history_cancellable") return readFails ? Promise.reject(readFailure) : Promise.resolve({});
      return Promise.reject(closeFailure);
    });
    await expect(invokeDesktopHistoryWait(invoke, {request: {}}, new AbortController().signal))
      .rejects.toBe(readFails ? readFailure : closeFailure);
  }
});

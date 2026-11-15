/** One invocation receipt. Timeout never cancels or replays the accepted work. */
export interface RuntimePluginWork {
  readonly promise: Promise<void>;
  state: "pending" | "fulfilled" | "rejected";
  reason?: unknown;
}

export function invokeRuntimePluginWork(run: () => void | Promise<void>): RuntimePluginWork {
  let promise: Promise<void>;
  try { promise = Promise.resolve(run()); }
  catch (reason) { promise = Promise.reject(reason); }
  const work: RuntimePluginWork = {promise, state: "pending"};
  void promise.then(() => { work.state = "fulfilled"; }, reason => {
    work.state = "rejected"; work.reason = reason;
  });
  return work;
}

/** The deadline covers the complete phase, including all sequential disposers. */
export function awaitRuntimePluginWork(work: RuntimePluginWork, deadline: number,
  timeout: () => Error): Promise<void> {
  const remaining = deadline - Date.now();
  if (remaining <= 0) return Promise.reject(timeout());
  if (work.state === "fulfilled") return Promise.resolve();
  if (work.state === "rejected") return Promise.reject(work.reason);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { reject(timeout()); }, remaining);
    void work.promise.then(() => { clearTimeout(timer); resolve(); }, reason => {
      clearTimeout(timer); reject(reason);
    });
  });
}

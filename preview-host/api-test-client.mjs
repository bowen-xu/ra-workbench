import {spawn} from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import {createInterface} from "node:readline";

/** Launch a version-matched Core application and use its public IPC commands. */
export async function launchWorkbenchApiTestHost({
  executable,
  dataDirectory,
  pluginDirectory,
  expectedVersion,
  environment = process.env,
  timeoutMs = 60_000,
}) {
  if (typeof executable !== "string" || typeof dataDirectory !== "string"
    || !path.isAbsolute(executable) || !path.isAbsolute(dataDirectory) || !expectedVersion
    || path.dirname(path.resolve(dataDirectory)) === path.resolve(dataDirectory)
    || (pluginDirectory !== undefined && !path.isAbsolute(pluginDirectory))) {
    throw new Error("workbench_api_test_host_configuration_invalid");
  }
  await fs.mkdir(dataDirectory, {recursive: true});
  const emptyDirectory = pluginDirectory === undefined
    ? await fs.mkdtemp(path.join(dataDirectory, ".empty-plugins-")) : undefined;
  const child = spawn(executable, ["--workbench-api-test-host"], {
    stdio: ["pipe", "pipe", "pipe"],
    env: {...environment,
      RESEARCH_ASSISTANT_DEVELOPMENT_DATA_DIRECTORY: dataDirectory,
      RESEARCH_ASSISTANT_CREDENTIALS_PATH: path.join(dataDirectory, "credentials.json"),
      RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY: pluginDirectory ?? emptyDirectory,
    },
  });
  let stderr = "";
  let sequence = 0;
  let closing = false;
  let stopped = false;
  let closingPromise;
  let failure;
  let resolveReady;
  let rejectReady;
  const pending = new Map();
  const ready = new Promise((resolve, reject) => {resolveReady = resolve; rejectReady = reject;});
  const exited = new Promise(resolve => child.once("close", (code, signal) => {
    stopped = true;
    resolve({code, signal});
    if (!closing || pending.size) {
      fail(new Error(`workbench_api_test_host_exited: code=${code} signal=${signal}\n${stderr}`));
    }
  }));
  const startupTimer = setTimeout(() => fail(new Error("workbench_api_test_host_startup_timeout\n" + stderr)), timeoutMs);

  function fail(error) {
    failure ??= error;
    clearTimeout(startupTimer);
    rejectReady(error);
    for (const request of pending.values()) {
      clearTimeout(request.timer);
      request.reject(error);
    }
    pending.clear();
    if (!stopped) child.kill("SIGTERM");
  }

  child.once("error", fail);
  child.stdin.on("error", fail);
  child.stdout.on("error", fail);
  child.stderr.on("error", fail);
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", chunk => {stderr = (stderr + chunk).slice(-16_384);});
  const output = createInterface({input: child.stdout});
  output.on("line", line => {
    let frame;
    try {frame = JSON.parse(line);} catch {return;}
    if (frame?.jsonrpc !== "2.0") return;
    if (frame.method === "workbench.ready") {
      if (frame.params?.protocolVersion !== 1 || frame.params.version !== expectedVersion) {
        fail(new Error("workbench_api_test_host_version_mismatch"));
        return;
      }
      clearTimeout(startupTimer);
      resolveReady(frame.params);
      return;
    }
    const request = pending.get(frame.id);
    if (!request || (("result" in frame) === ("error" in frame))
      || ("error" in frame && (!frame.error || typeof frame.error !== "object"
        || !Number.isInteger(frame.error.code) || typeof frame.error.message !== "string"))) {
      fail(new Error("workbench_api_test_host_response_invalid"));
      return;
    }
    pending.delete(frame.id);
    clearTimeout(request.timer);
    if (frame.error) request.reject(frame.error.data ?? frame.error);
    else request.resolve(frame.result);
  });
  try {await ready;} catch (error) {
    await exited;
    if (emptyDirectory) await fs.rm(emptyDirectory, {recursive: true});
    throw error;
  }

  async function invoke(command, arguments_ = {}) {
    if (failure) throw failure;
    if (closing || stopped) throw new Error("workbench_api_test_host_closed");
    const id = ++sequence;
    const frame = JSON.stringify({jsonrpc: "2.0", id, method: command, params: arguments_}) + "\n";
    let resolve;
    let reject;
    const promise = new Promise((accept, refuse) => {resolve = accept; reject = refuse;});
    const timer = setTimeout(() => {
      reject(new Error(`workbench_api_test_host_request_timeout: requestId=${id} command=${command}`));
    }, timeoutMs);
    pending.set(id, {resolve, reject, timer, promise});
    child.stdin.write(frame, error => {
      if (error) fail(error);
    });
    return promise;
  }

  async function finishClose() {
    closing = true;
    await Promise.allSettled([...pending.values()].map(request => request.promise));
    child.stdin.end();
    const timer = setTimeout(() => fail(new Error("workbench_api_test_host_shutdown_timeout\n" + stderr)), timeoutMs);
    try {
      const result = await exited;
      if (failure) throw failure;
      if (result.code !== 0) throw new Error(`workbench_api_test_host_shutdown_failed: code=${result.code} signal=${result.signal}\n${stderr}`);
    } finally {
      clearTimeout(timer);
      output.close();
      if (emptyDirectory) await fs.rm(emptyDirectory, {recursive: true, force: true});
    }
  }

  function close() {
    closingPromise ??= finishClose();
    return closingPromise;
  }

  return Object.freeze({invoke, close, processId: child.pid, version: expectedVersion});
}

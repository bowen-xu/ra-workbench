import {spawnSync} from "node:child_process";
import {dirname, resolve} from "node:path";
import {fileURLToPath, pathToFileURL} from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/** Core's own tests use the caller's environment and its selected Cargo target. */
export async function runNativeTests(arguments_, {
  root = repositoryRoot,
  environment = process.env,
  spawn = spawnSync,
} = {}) {
  const args = arguments_[0] === "--" ? arguments_.slice(1) : arguments_;
  const result = spawn("cargo", ["test", ...args], {cwd: root, env: environment, stdio: "inherit"});
  if (result.error) throw result.error;
  return result.status ?? 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { process.exitCode = await runNativeTests(process.argv.slice(2)); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}

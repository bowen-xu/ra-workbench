import fs from "node:fs/promises";
import path from "node:path";
import {fileURLToPath} from "node:url";
import {withCachedBuild} from "../.dependencies/sdk/tooling/build-artifacts.mjs";
import {runBuildCommand} from "../.dependencies/sdk/tooling/build-command.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const receipt = await withCachedBuild({root, kind: "workbench-native", inputs: ["apps/desktop", "packages/agent-core", "packages/data-runtime", "packages/docling-runtime", "scripts", "package.json", "pnpm-lock.yaml", "pnpm-workspace.yaml", ".cargo"]}, async context => {
  const result = await runBuildCommand("core-native", "cargo", ["build", "--locked", "--manifest-path",
    "apps/desktop/src-tauri/Cargo.toml", "--no-default-features", "--bin", "research-assistant", "--bin", "note-pdf-worker", "--message-format", "json"],
  {cwd: root, env: process.env}, path.join(context.logDirectory, "cargo.log"));
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error("workbench_native_build_failed: cargo exited with " + result.status);
  await fs.mkdir(context.output, {recursive: true});
  const compiled = result.stdout.split("\n").filter(Boolean).map(line => JSON.parse(line));
  for (const name of ["research-assistant", "note-pdf-worker"]) {
    const source = compiled.find(value => value.reason === "compiler-artifact" && value.target.name === name && value.executable)?.executable;
    if (!source) throw new Error("workbench_native_artifact_missing: " + name);
    const target = path.join(context.output, path.basename(source));
    await fs.copyFile(source, target);
    await fs.chmod(target, (await fs.stat(source)).mode & 0o777);
  }
  return {status: "passed"};
});
console.log(JSON.stringify({status: receipt.status, output: receipt.output, cacheHit: receipt.cacheHit}));

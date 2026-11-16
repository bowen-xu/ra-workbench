import fs from "node:fs/promises";
import path from "node:path";
import {createHash} from "node:crypto";
import {fileURLToPath} from "node:url";
import {withCachedBuild} from "../.dependencies/sdk/tooling/build-artifacts.mjs";
import {runBuildCommand} from "../.dependencies/sdk/tooling/build-command.mjs";
import {temporaryApplicationPath} from "./development-application.mjs";
import {DEVELOPMENT_IDENTIFIER} from "./release-environments.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
if (process.platform !== "darwin") throw new Error("workbench_api_test_host_bundle_platform_unsupported");
const receipt = await withCachedBuild({root, kind: "api-test-host", native: true,
  exclude: ["apps/web/public/dictionary"],
  inputs: ["apps", "packages", "scripts", "patches", "preview-host", ".cargo", "package.json",
    "pnpm-lock.yaml", "pnpm-workspace.yaml", "tsconfig.base.json"],
}, async context => {
  const environment = {...process.env, RESEARCH_ASSISTANT_BUILTIN_PLUGINS: "none"};
  delete environment.RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY;
  const build = await runBuildCommand("core-api-test-host", "pnpm", ["bundle:dev"],
    {cwd: root, env: environment}, path.join(context.logDirectory, "bundle.log"));
  if (build.error) throw build.error;
  if (build.status !== 0) throw new Error("workbench_api_test_host_bundle_failed: " + build.status);
  const application = temporaryApplicationPath(root);
  const archive = "workbench-api-test-host.tar.gz";
  await fs.mkdir(context.output, {recursive: true});
  const packed = await runBuildCommand("core-api-test-host", "tar",
    ["-czf", path.join(context.output, archive), "-C", path.dirname(application), path.basename(application)],
    {cwd: root, env: environment}, path.join(context.logDirectory, "archive.log"));
  if (packed.error) throw packed.error;
  if (packed.status !== 0) throw new Error("workbench_api_test_host_archive_failed: " + packed.status);
  const metadata = JSON.parse(await fs.readFile(path.join(root, "package.json"), "utf8"));
  const bytes = await fs.readFile(path.join(context.output, archive));
  await fs.writeFile(path.join(context.output, "api-test-host.json"), JSON.stringify({
    formatVersion: 1, protocolVersion: 1, version: metadata.version,
    platform: process.platform, arch: process.arch, identifier: DEVELOPMENT_IDENTIFIER,
    archive, sha256: createHash("sha256").update(bytes).digest("hex"),
    executable: path.posix.join(path.basename(application), "Contents/MacOS/research-assistant"),
  }, null, 2) + "\n");
  return {status: "passed", version: metadata.version};
});
console.log(JSON.stringify({status: receipt.status, output: receipt.output,
  cacheHit: receipt.cacheHit, buildKey: receipt.buildKey, artifactKey: receipt.artifactKey}));

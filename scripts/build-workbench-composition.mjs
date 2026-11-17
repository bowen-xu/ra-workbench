import fs from "node:fs/promises";
import path from "node:path";
import {spawnSync} from "node:child_process";
import {fileURLToPath, pathToFileURL} from "node:url";
import {readWorkbenchBuildPlan} from "./workbench-build-plan.mjs";
import {findExcludedWorkbenchPluginSources} from "./workbench-boundaries.mjs";
import {logicalProjectPath} from "./project-layout.mjs";
import {materializeWorkbenchWeb} from "./materialize-workbench-web.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
function run(command, args, cwd = root, env = process.env) {
  const result = spawnSync(command, args, {cwd, env, stdio: "inherit"});
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`workbench_build_failed: ${command} exited with ${result.status}`);
}

export async function buildWorkbenchWeb({checkBoundaries = false, planOnly = false, input = "none"} = {}) {
  const plan = await readWorkbenchBuildPlan(input);
  if (planOnly) return plan;
  // Explicit composition builds also validate the source boundary.
  if (checkBoundaries) run("node", ["scripts/check-workbench-boundaries.mjs"]);
  const {withCachedBuild} = await import("../.dependencies/sdk/tooling/build-artifacts.mjs");
  const receipt = await withCachedBuild({
    root, kind: "workbench-web",
    exclude: ["apps/web/public/dictionary"],
    inputs: ["apps/web", "packages/ui", "scripts", "docs/architecture", "package.json",
      "pnpm-lock.yaml", "pnpm-workspace.yaml", "tsconfig.base.json"],
  }, async context => {
    const output = context.output;
    await fs.mkdir(output, {recursive: true});
    run("node", ["scripts/prepare-note-query.mjs"]);
    run("node", ["scripts/prepare-dictionary.mjs"]);
    run("pnpm", ["exec", "tsc", "-b"], path.join(root, "apps/web"));
    run("pnpm", ["exec", "vite", "build", "--outDir", path.join(output, "web"), "--emptyOutDir"],
      path.join(root, "apps/web"), {...process.env, RESEARCH_ASSISTANT_BUILTIN_PLUGINS: "none"});
    const evidence = JSON.parse(await fs.readFile(path.join(output, "web/workbench-composition.json"), "utf8"));
    const config = path.join(output, "tsconfig.json");
    const roots = ["apps/web/src/env.d.ts", "apps/web/src/main.selected.tsx", ...(plan.runtimeSdkTypeRoots ?? [])];
    await fs.writeFile(config, JSON.stringify({extends: path.join(root, "tsconfig.base.json"), compilerOptions: {
      jsx: "react-jsx", lib: ["ES2022", "DOM", "DOM.Iterable"], noEmit: true, resolveJsonModule: true,
    }, files: roots.map(file => path.join(root, file))}, null, 2));
    run("node", [path.join(root, "apps/web/node_modules/typescript/bin/tsc"), "-p", config, "--pretty", "false"]);
    const list = spawnSync("node", [path.join(root, "apps/web/node_modules/typescript/bin/tsc"), "-p", config, "--listFilesOnly"], {cwd: root, encoding: "utf8"});
    if (list.status !== 0) throw new Error(list.stderr || list.stdout);
    const sources = list.stdout.split(/\r?\n/u).filter(file => file.startsWith(root));
    const policy = JSON.parse(await fs.readFile(path.join(root, "docs/architecture/workbench-boundaries.policy.json"), "utf8"));
    const excluded = findExcludedWorkbenchPluginSources(sources.map(file => logicalProjectPath(root, file)), evidence.plugins, policy.contracts);
    if (excluded.length) throw new Error(`plugin_type_closure_violation：\n${excluded.join("\n")}`);
    await fs.writeFile(path.join(output, "typescript-sources.json"), JSON.stringify(sources, null, 2));
    if (plan.runtimeSdkTypeRoots) await fs.writeFile(path.join(output, "typescript-roots.json"), JSON.stringify({roots, runtimeSdkTypeRoots: plan.runtimeSdkTypeRoots}, null, 2));
    return {status: "passed", plugins: evidence.plugins,
      cargo: ["--no-default-features", ...(evidence.cargoFeatures.length ? ["--features", evidence.cargoFeatures.join(",")] : [])]};
  });
  await materializeWorkbenchWeb({root, receipt});
  return receipt;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const planOnly = process.argv.includes("--plan");
  const args = process.argv.slice(2).filter(value => value !== "--plan");
  if (args.length !== 2 || args[0] !== "--plugins" || !args[1]) throw new Error("Usage: pnpm build:workbench [--plan] --plugins none");
  const result = await buildWorkbenchWeb({checkBoundaries: true, planOnly, input: args[1]});
  console.log(JSON.stringify(planOnly ? result : {plugins: result.plugins, cargo: result.cargo,
    evidence: result.output, output: result.output, receipt: result.receipt, cacheHit: result.cacheHit}));
}

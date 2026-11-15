import fs from "node:fs/promises";
import path from "node:path";
import {spawnSync} from "node:child_process";
import {fileURLToPath} from "node:url";
import {readWorkbenchBuildPlan} from "./workbench-build-plan.mjs";
import {findExcludedWorkbenchPluginSources} from "./workbench-boundaries.mjs";
import {logicalProjectPath} from "./project-layout.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const planOnly = process.argv.includes("--plan"), args = process.argv.slice(2).filter(value => value !== "--plan");
if (args.length !== 2 || args[0] !== "--plugins" || !args[1]) throw new Error("Usage: pnpm build:workbench [--plan] --plugins none");
const input = args[1];
const plan = await readWorkbenchBuildPlan(input), ids = plan.plugins;
if (planOnly) {console.log(JSON.stringify(plan)); process.exit(0);}
const run = (cmd, argv, cwd = root, env = process.env) => {
  const result = spawnSync(cmd, argv, {cwd, env, stdio: "inherit"});
  if (result.status !== 0) process.exit(result.status ?? 1);
};
const workbenchRoot = path.dirname(await fs.realpath(path.join(root, "apps")));
const output = path.join(workbenchRoot, ".tmp/workbench-compositions", ids.join("+") || "core");
await fs.mkdir(output, {recursive: true});
run("node", ["scripts/check-workbench-boundaries.mjs"]);
run("node", ["scripts/prepare-note-query.mjs"]);
run("node", ["scripts/prepare-dictionary.mjs"]);
run("pnpm", ["exec", "vite", "build", "--outDir", path.join(output, "web"), "--emptyOutDir"], path.join(root, "apps/web"), {...process.env, RESEARCH_ASSISTANT_BUILTIN_PLUGINS: input});
const evidence = JSON.parse(await fs.readFile(path.join(output, "web/workbench-composition.json"), "utf8"));
const config = path.join(output, "tsconfig.json");
const roots = ["apps/web/src/env.d.ts", "apps/web/src/main.selected.tsx",
  ...(plan.runtimeSdkTypeRoots ?? [])];
await fs.writeFile(config, JSON.stringify({extends: path.join(root, "tsconfig.base.json"), compilerOptions: {
  jsx: "react-jsx", lib: ["ES2022", "DOM", "DOM.Iterable"], noEmit: true, resolveJsonModule: true,
}, files: roots.map(file => path.join(root, file))}, null, 2));
run("node", [path.join(root, "apps/web/node_modules/typescript/bin/tsc"), "-p", config, "--pretty", "false"]);
const list = spawnSync("node", [path.join(root, "apps/web/node_modules/typescript/bin/tsc"), "-p", config, "--listFilesOnly"], {cwd: root, encoding: "utf8"});
if (list.status !== 0) throw new Error(list.stderr || list.stdout);
const sources = list.stdout.split(/\r?\n/u).filter(file => file.startsWith(root));
const boundaryPolicy = JSON.parse(await fs.readFile(path.join(root, "docs/architecture/workbench-boundaries.policy.json"), "utf8"));
const excluded = findExcludedWorkbenchPluginSources(sources.map(file => logicalProjectPath(root, file)), evidence.plugins, boundaryPolicy.contracts);
if (excluded.length) throw new Error(`plugin_type_closure_violation：\n${excluded.join("\n")}`);
await fs.writeFile(path.join(output, "typescript-sources.json"), JSON.stringify(sources, null, 2));
if (plan.runtimeSdkTypeRoots) await fs.writeFile(path.join(output, "typescript-roots.json"), JSON.stringify({
  roots, runtimeSdkTypeRoots: plan.runtimeSdkTypeRoots,
}, null, 2));
console.log(JSON.stringify({plugins: evidence.plugins, cargo: ["--no-default-features", ...(evidence.cargoFeatures.length ? ["--features", evidence.cargoFeatures.join(",")] : [])], evidence: output}));

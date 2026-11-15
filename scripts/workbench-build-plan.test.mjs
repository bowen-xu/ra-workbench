import assert from "node:assert/strict";
import {test} from "node:test";
import {spawnSync} from "node:child_process";
import {readWorkbenchBuildPlan} from "./workbench-build-plan.mjs";

test("builds only the dynamic Core and its actual SDK facade producers", async () => {
  const plan = await readWorkbenchBuildPlan("none");
  assert.deepEqual(plan.plugins, []);
  assert.deepEqual(plan.cargoFeatures, []);
  assert.ok(plan.runtimeSdkTypeRoots.includes("apps/web/src/core/public/noteDocuments.ts"));
  assert.ok(plan.runtimeSdkTypeRoots.includes("apps/web/src/core/public/layout.ts"));
  assert.ok(plan.runtimeSdkTypeRoots.every(source => source.startsWith("apps/web/src/core/public/") && source.endsWith(".ts")));
  assert.equal(new Set(plan.runtimeSdkTypeRoots).size, plan.runtimeSdkTypeRoots.length);
  for (const input of ["all", "writing_projects", "unknown"])
    await assert.rejects(readWorkbenchBuildPlan(input), /plugin_static_selection_removed/u);
});

test("keeps JSON-only Core plan mode", async () => {
  const result = spawnSync(process.execPath, ["scripts/build-workbench-composition.mjs", "--plan", "--plugins", "none"], {encoding: "utf8"});
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), await readWorkbenchBuildPlan("none"));
});

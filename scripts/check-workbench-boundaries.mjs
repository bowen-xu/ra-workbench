import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { collectWorkbenchImports, checkWorkbenchBoundaries } from "./workbench-boundaries.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = path => JSON.parse(readFileSync(resolve(root, path), "utf8"));
const policy = read("docs/architecture/workbench-boundaries.policy.json");
for (const file of [...policy.publicEntries, ...policy.contracts, ...(policy.browserTestEntries ?? []), ...(policy.testAssemblyEntries ?? [])])
  if (!existsSync(resolve(root, file))) throw new Error(`Missing public boundary file: ${file}`);
const report = checkWorkbenchBoundaries(collectWorkbenchImports(root),
  policy,
  read("docs/architecture/workbench-boundaries.baseline.json"));
if (process.argv.includes("--json")) process.stdout.write(JSON.stringify(report, null, 2) + "\n");
else {
  for (const edge of report.violations)
    process.stderr.write(`${edge.from}:${edge.line} ${edge.reason}: ${edge.target} [${edge.names}]\n`);
  for (const item of report.stale)
    process.stderr.write(`Remove resolved migration exception: ${item.key}\n`);
  process.stdout.write(`Workbench boundaries: ${report.ok ? "pass" : "FAIL"}; ${report.checkedImports} imports; ${report.legacyReferences} exact legacy references.\n`);
}
if (!report.ok) process.exitCode = 1;

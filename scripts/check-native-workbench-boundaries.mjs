import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checkNativeBoundaries } from "./native-workbench-boundaries.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = file => JSON.parse(readFileSync(resolve(root, file), "utf8"));
const report = checkNativeBoundaries(root,
  read("docs/architecture/native-workbench-boundaries.policy.json"),
  read("docs/architecture/native-workbench-boundaries.exceptions.json"));
if (process.argv.includes("--json")) process.stdout.write(JSON.stringify(report, null, 2) + "\n");
else {
  for (const edge of report.violations)
    process.stderr.write(`${edge.from}:${edge.line ?? 1} ${edge.reason}: ${edge.target}\n`);
  for (const item of report.unsupported)
    process.stderr.write(`${item.from}:${item.line ?? 1} unsupported: ${item.reason}\n`);
  for (const item of report.stale)
    process.stderr.write(`Native boundary policy: ${item.reason}: ${item.key}\n`);
  for (const item of report.pending)
    process.stderr.write(`Native boundary registration pending: ${item}\n`);
  process.stdout.write(`Native workbench boundaries: ${report.ok ? "pass" : "FAIL"}; ${report.checkedConsumers} consumers; ${report.checkedReferences} literal references.\n`);
}
if (!report.ok) process.exitCode = 1;

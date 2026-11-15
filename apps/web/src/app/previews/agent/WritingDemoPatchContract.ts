import type { DemoChange } from "../../../core/features/agent/WritingDiffFixtures";
import { formatSourceLink } from "@research-assistant/core/source";

/** Provider fixture data uses the same v2 text contract as new native turns. */
export function writingDemoPatch(projectId: string, changes: readonly Pick<DemoChange, "path" | "kind" | "before" | "after" | "beforeBytes" | "afterBytes">[]) {
  // Binary/large delivery fixtures represent existing execution-copy output.
  // They cannot be fabricated as successful text-patch calls or oversized deltas.
  if (changes.some(change => change.beforeBytes || change.afterBytes || change.before.length + change.after.length > 24_000)) return {
    name: "research_exec_command", arguments: { command: "mock-generate-delivery-fixtures" },
    output: { state: "finished", exitCode: 0, output: "Execution-copy delivery fixtures prepared." },
  };
  const path = (value: string) => formatSourceLink({ path: ["writings", projectId, ...value.split("/")] });
  const lines = (text: string, prefix: string) => {
    const normalized = text.replace(/\r\n/g, "\n");
    return (normalized.endsWith("\n") ? normalized.slice(0, -1) : normalized).split("\n").map(line => prefix + line).join("\n");
  };
  return {
    name: "research_apply_patch",
    arguments: { patch: ["*** Begin Patch", ...changes.map(change => {
      if (change.kind === "deleted") return `*** Delete File: ${path(change.path)}`;
      if (change.kind === "added") return `*** Add File: ${path(change.path)}\n${lines(change.after, "+")}`;
      return `*** Update File: ${path(change.path)}\n@@\n${lines(change.before, "-")}\n${lines(change.after, "+")}`;
    }), "*** End Patch"].join("\n") },
    output: { status: "complete", targets: changes.map(change => ({ path: path(change.path), name: change.path.split("/").at(-1), status: "applied", message: "Authorized execution copy updated." })) },
  };
}

import {describe, expect, it} from "vitest";
import {readDevelopmentWorkbenchSelection, developmentWorkbenchArguments} from "./workbench-development-arguments.mjs";

describe("dynamic development arguments", () => {
  it("always selects the empty Core compilation", () => {
    expect(readDevelopmentWorkbenchSelection("/workspace", {})).toEqual({plugins: [], cargoFeatures: []});
  });
  it("rejects removed static selections and runtime opt-out", () => {
    for (const value of ["all", "writing_projects", "unknown"])
      expect(() => readDevelopmentWorkbenchSelection("/workspace", {RESEARCH_ASSISTANT_BUILTIN_PLUGINS: value})).toThrow("plugin_static_selection_removed");
    expect(() => readDevelopmentWorkbenchSelection("/workspace", {RESEARCH_ASSISTANT_RUNTIME_PLUGINS: "0"})).toThrow("plugin_static_selection_removed");
  });
  it("preserves Cargo and application arguments", () => {
    expect(developmentWorkbenchArguments(["dev"], undefined, ["--no-watch", "--", "--jobs", "1", "--", "synthetic.txt"]))
      .toEqual(["dev", "--no-watch", "--", "--no-default-features", "--jobs", "1", "--", "synthetic.txt"]);
    expect(developmentWorkbenchArguments(["build"], undefined, [], {cargo: true})).toEqual(["build", "--no-default-features"]);
  });
  it("rejects feature overrides before building", () => {
    for (const args of [["--features", "removed-domain"], ["--features=removed-domain"], ["-f", "removed-domain"], ["--no-default-features"]])
      expect(() => developmentWorkbenchArguments(["dev"], undefined, args)).toThrow("plugin_build_selection_conflict");
  });
});

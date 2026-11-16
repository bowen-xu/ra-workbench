import {describe, expect, it} from "vitest";
import {createRuntimeDesktopDevArguments} from "./dev-desktop.mjs";

describe("dynamic desktop development", () => {
  it("builds Core on the local protocol with no static domain features", () => {
    const args = createRuntimeDesktopDevArguments("/workspace", ["--no-watch"]);
    expect(args).toContain("development-data");
    expect(args).toContain("--no-dev-server");
    expect(args).toContain("--no-default-features");
    expect(args.slice(-3)).toEqual(["--no-watch", "--", "--no-default-features"]);
    expect(JSON.parse(args.find(value => value.startsWith("{")))).toEqual({build: {
      beforeDevCommand: "node ../../scripts/build-workbench-composition.mjs --plugins none",
      devUrl: null, frontendDist: "/workspace/apps/web/dist",
    }});
  });
});

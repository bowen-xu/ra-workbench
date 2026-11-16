import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import {test} from "node:test";
import {withCachedBuild} from "../.dependencies/sdk/tooling/build-artifacts.mjs";
import {NATIVE_BUILD_EXCLUDES} from "./native-build-inputs.mjs";

test("a source archive publishes after Tauri schema generation and tracks authored capabilities", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "workbench-native-inputs-"));
  try {
    const desktop = path.join(root, "apps/desktop/src-tauri");
    await fs.mkdir(path.join(desktop, "src"), {recursive: true});
    await fs.mkdir(path.join(desktop, "capabilities"));
    await fs.writeFile(path.join(desktop, "Cargo.toml"), "[package]\nname='fixture'\nversion='0.1.0'\n");
    await fs.writeFile(path.join(desktop, "src/lib.rs"), "pub fn fixture() {}\n");
    const capability = path.join(desktop, "capabilities/default.json");
    await fs.writeFile(capability, JSON.stringify({permissions: []}));
    let builds = 0;
    const build = async ({output}) => {
      builds++;
      await fs.mkdir(path.join(desktop, "gen/schemas"), {recursive: true});
      for (const name of ["acl-manifests", "capabilities", "desktop-schema", "macOS-schema"]) {
        await fs.writeFile(path.join(desktop, "gen/schemas", name + ".json"), "{}\n");
      }
      await fs.mkdir(output, {recursive: true});
      await fs.writeFile(path.join(output, "host"), "synthetic compiled output");
      return {status: "passed"};
    };
    const options = {root, kind: "workbench-native", toolchains: false,
      inputs: ["apps/desktop"], exclude: NATIVE_BUILD_EXCLUDES};
    const first = await withCachedBuild(options, build);
    assert.equal(first.status, "passed");
    assert.equal((await withCachedBuild(options, build)).cacheHit, true);
    await fs.writeFile(capability, JSON.stringify({permissions: ["fixture:read"]}));
    const changed = await withCachedBuild(options, build);
    assert.notEqual(changed.buildKey, first.buildKey);
    assert.equal(builds, 2);
  } finally {
    await fs.rm(root, {recursive: true, force: true});
  }
});

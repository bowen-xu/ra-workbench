import {mkdir, mkdtemp, readFile, rm, writeFile} from "node:fs/promises";
import {join} from "node:path";
import {afterEach, describe, expect, it, vi} from "vitest";
import {prepareNativeTestEnvironment, runNativeTests} from "./run-native-tests.mjs";

const roots = [];
async function fixture() {
  await mkdir(new URL("../.tmp/", import.meta.url), {recursive: true});
  const root = await mkdtemp(new URL("../.tmp/native-test-runner-", import.meta.url));
  roots.push(root);
  const writing = join(root, ".tmp", "writing-packages");
  const literature = join(root, ".tmp", "literature-package");
  await mkdir(writing, {recursive: true});
  await mkdir(literature, {recursive: true});
  return {root, writing, literature};
}
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, {recursive: true, force: true}))); });

describe("Core tests with independently built native plugins", () => {
  it("reuses explicit fixture directories without rebuilding or changing the caller environment", async () => {
    const {root, writing, literature} = await fixture();
    const environment = {RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES: writing,
      RESEARCH_ASSISTANT_TEST_LITERATURE_PACKAGE: literature, SYNTHETIC_TEST_VALUE: "retained"};
    const build = vi.fn();
    expect(await prepareNativeTestEnvironment({root, environment, build})).toEqual(environment);
    expect(build).not.toHaveBeenCalled();
  });
  it("builds canonical native profiles and retains the original writing package layout", async () => {
    const {root, writing, literature} = await fixture();
    await writeFile(join(writing, "manifest.json"), "synthetic writing manifest");
    const build = vi.fn(async id => ({status: "passed", packageRoot: id === "writing_projects" ? writing : literature}));
    const environment = {};
    const selected = await prepareNativeTestEnvironment({root, environment, build});
    expect(build.mock.calls).toEqual([["writing_projects", ["--offline", "--project-files"]], ["literature_library", ["--offline"]]]);
    expect(await readFile(join(selected.RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES, "package", "manifest.json"), "utf8")).toBe("synthetic writing manifest");
    expect(selected.RESEARCH_ASSISTANT_TEST_LITERATURE_PACKAGE).toBe(literature);
    expect(environment).toEqual({});
  });
  it("fails before Cargo when a selected fixture is outside repository temporary storage", async () => {
    const {root} = await fixture();
    const build = vi.fn();
    await expect(prepareNativeTestEnvironment({root, environment: {RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES: root}, build}))
      .rejects.toThrow("native_test_packages_outside_repository_tmp");
    expect(build).not.toHaveBeenCalled();
  });
  it("preserves Cargo test filters, exact flags, fixture environment and failure exit status", async () => {
    const prepare = vi.fn(async () => ({SYNTHETIC_TEST_VALUE: "fixtures"}));
    const spawn = vi.fn(() => ({status: 101}));
    const args = ["--manifest-path", "apps/desktop/src-tauri/Cargo.toml", "migration::upgrade", "--", "--exact"];
    expect(await runNativeTests(["--", ...args], {root: "/repository", environment: {}, prepare, spawn})).toBe(101);
    expect(spawn).toHaveBeenCalledWith("cargo", ["test", ...args], {
      cwd: "/repository", env: {SYNTHETIC_TEST_VALUE: "fixtures"}, stdio: "inherit",
    });
  });
  it("does not start Cargo after fixture preparation fails", async () => {
    const prepare = vi.fn(async () => { throw new Error("native fixture build failed"); });
    const spawn = vi.fn();
    await expect(runNativeTests([], {prepare, spawn})).rejects.toThrow("native fixture build failed");
    expect(spawn).not.toHaveBeenCalled();
  });
});

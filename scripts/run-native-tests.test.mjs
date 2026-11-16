import {describe, expect, it, vi} from "vitest";
import {runNativeTests} from "./run-native-tests.mjs";

describe("Core native test entry", () => {
  it("runs the selected Cargo target with the original environment and failure status", async () => {
    const environment = {SYNTHETIC_TEST_VALUE: "Core"};
    const spawn = vi.fn(() => ({status: 101}));
    const args = ["--manifest-path", "apps/desktop/src-tauri/Cargo.toml", "migration::upgrade", "--", "--exact"];
    expect(await runNativeTests(["--", ...args], {root: "/repository", environment, spawn})).toBe(101);
    expect(spawn).toHaveBeenCalledExactlyOnceWith("cargo", ["test", ...args], {
      cwd: "/repository", env: environment, stdio: "inherit",
    });
  });
  it("retains the original process startup error", async () => {
    const error = new Error("synthetic Cargo startup failure");
    await expect(runNativeTests([], {spawn: () => ({error})})).rejects.toBe(error);
  });
});

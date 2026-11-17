import {afterEach, expect, it, vi} from "vitest";

const dependencies = vi.hoisted(() => ({build: vi.fn(), cargo: vi.fn(() => ({status: 0}))}));
vi.mock("./build-workbench-composition.mjs", () => ({buildWorkbenchWeb: dependencies.build}));
vi.mock("node:child_process", () => ({spawnSync: dependencies.cargo}));
import {prepareDesktopDevelopment} from "./prepare-desktop-development.mjs";

afterEach(() => vi.clearAllMocks());

it("prepares frontend before Cargo and preserves Tauri's exact environment", async () => {
  const environment = {TAURI_CONFIG: '{"identifier":"synthetic.dev"}'};
  await prepareDesktopDevelopment({root: "/synthetic", environment});
  expect(dependencies.build).toHaveBeenCalledWith({checkBoundaries: true, input: "none"});
  if (process.platform === "darwin") {
    expect(dependencies.cargo).toHaveBeenCalledWith("cargo", ["build", "--locked", "--manifest-path",
      "apps/desktop/src-tauri/Cargo.toml", "--bin", "note-pdf-worker", "--features",
      "development-data", "--no-default-features"], {cwd: "/synthetic", env: environment, stdio: "inherit"});
    expect(dependencies.build.mock.invocationCallOrder[0]).toBeLessThan(dependencies.cargo.mock.invocationCallOrder[0]);
  }
});

it("does not start the worker when frontend preparation fails", async () => {
  dependencies.build.mockRejectedValueOnce(new Error("synthetic frontend failure"));
  await expect(prepareDesktopDevelopment()).rejects.toThrow("synthetic frontend failure");
  expect(dependencies.cargo).not.toHaveBeenCalled();
});

it.skipIf(process.platform !== "darwin")("propagates worker compilation failure", async () => {
  dependencies.cargo.mockReturnValueOnce({status: 9});
  await expect(prepareDesktopDevelopment()).rejects.toThrow("note_pdf_worker_build_failed: cargo exited with 9");
});

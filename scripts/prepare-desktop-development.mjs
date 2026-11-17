import {spawnSync} from "node:child_process";
import path from "node:path";
import {fileURLToPath, pathToFileURL} from "node:url";
import {buildWorkbenchWeb} from "./build-workbench-composition.mjs";

/** Tauri's waiting hook shares its exact configuration with both Cargo builds. */
export async function prepareDesktopDevelopment({
  root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), ".."),
  environment = process.env,
} = {}) {
  const started = performance.now();
  console.log("[desktop] Preparing Core frontend");
  await buildWorkbenchWeb({checkBoundaries: true, input: "none"});
  console.log(`[desktop] Core frontend ready in ${((performance.now() - started) / 1000).toFixed(2)}s`);
  if (process.platform === "darwin") {
    const workerStarted = performance.now();
    console.log("[desktop] Checking native PDF worker with Tauri development configuration");
    const result = spawnSync("cargo", ["build", "--locked", "--manifest-path",
      "apps/desktop/src-tauri/Cargo.toml", "--bin", "note-pdf-worker",
      "--features", "development-data", "--no-default-features"],
    {cwd: root, env: environment, stdio: "inherit"});
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`note_pdf_worker_build_failed: cargo exited with ${result.status}`);
    console.log(`[desktop] Native PDF worker ready in ${((performance.now() - workerStarted) / 1000).toFixed(2)}s`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await prepareDesktopDevelopment();
}

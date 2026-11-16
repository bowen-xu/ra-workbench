import fs from "node:fs";
import { configDefaults, defineConfig } from "vitest/config";
import {workbenchSdkSourcePlugin} from "./apps/web/runtimeSdkPlugin.ts";

export default defineConfig({
  plugins: [workbenchSdkSourcePlugin()],
  test: {
    setupFiles: ["./tests/setup/browserNotesDatabase.ts", "./apps/web/src/core/features/agent/markdown/markdownWorker.testSetup.ts"],
    // Standalone UI templates and reference repositories own their test configs.
    exclude: [...configDefaults.exclude, ".codex/archives/**", ".tmp/**", ".dependencies/**", ...fs.readdirSync("scripts").filter(file => file.endsWith(".test.mjs") && /from ["']node:test["']/u.test(fs.readFileSync("scripts/" + file, "utf8"))).map(file => "scripts/" + file), ".pnpm-store/**", "tests/sdk-contracts/**", "docs/ui-templates/settings-template/**", "docs/references/projects/**"],
    // happy-dom owns browser storage. Node 25's global storage otherwise wins
    // global population and exposes an unusable object without a storage file.
    execArgv: process.allowedNodeEnvironmentFlags.has("--no-experimental-webstorage")
      ? ["--no-experimental-webstorage"]
      : [],
  },
});

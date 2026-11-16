import {defineConfig, devices} from "@playwright/test";
import {fileURLToPath} from "node:url";

const root = fileURLToPath(new URL("./", import.meta.url));
const port = process.env.PLAYWRIGHT_CORE_PORT ?? "4174";
export default defineConfig({
  testDir: fileURLToPath(new URL("tests/ui/", import.meta.url)),
  testMatch: "**/*.pw.ts",
  fullyParallel: false,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: "list",
  outputDir: fileURLToPath(new URL(".tmp/playwright-results/", import.meta.url)),
  use: {baseURL: `http://127.0.0.1:${port}`, trace: "retain-on-failure", screenshot: "only-on-failure", video: "off"},
  projects: [{name: "chromium", use: {...devices["Desktop Chrome"], viewport: {width: 1280, height: 720}}}],
  webServer: {
    cwd: root,
    command: `pnpm --config.verify-deps-before-run=false --filter @research-assistant/web dev --port ${port}`,
    url: `http://127.0.0.1:${port}`, reuseExistingServer: false, timeout: 120_000,
    gracefulShutdown: {signal: "SIGTERM", timeout: 5_000},
  },
});

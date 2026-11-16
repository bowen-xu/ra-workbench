import {createRequire} from "node:module";
import {defineConfig, mergeConfig} from "vitest/config";
import core from "./vitest.config.mts";

// Product acceptance explicitly opts into external plugin fixtures. The regular
// Core suite does not collect these tests or require business plugin checkouts.
const require = createRequire(new URL("./apps/web/package.json", import.meta.url));
const productCore = {...core, test: {...core.test, exclude: core.test.exclude.filter(pattern => pattern !== "tests/plugin-integration/**")}};
export default mergeConfig(productCore, defineConfig({
  plugins: [{
    name: "product-test-public-packages",
    enforce: "pre",
    resolveId(id) {
      if (/^@research-assistant\/(?:core|data|writing|workbench-sdk)(?:\/|$)/u.test(id)) return require.resolve(id);
    },
  }],
  test: {
    include: ["tests/plugin-integration/**/*.test.{ts,tsx}"],
    exclude: [],
  },
}));

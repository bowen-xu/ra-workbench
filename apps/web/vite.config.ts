import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";
import { createRequire } from "node:module";
import {workbenchCompositionPlugin} from "./workbenchCompositionPlugin";
import {runtimeSdkPlugin, workbenchSdkSourcePlugin} from "./runtimeSdkPlugin.ts";


export default defineConfig(({command}) => {
  const selection = "none";
  const runtime = command === "build" ? "1" : undefined;
  return {
    clearScreen: false,
    define: {
      __APP_VERSION__: JSON.stringify(process.env.npm_package_version ?? "0.0.0"),
      __RUNTIME_PLUGINS__: runtime === "1",
    },
    plugins: [react(), workbenchCompositionPlugin(selection), workbenchSdkSourcePlugin(), runtimeSdkPlugin(runtime, selection)],
    resolve: {
      alias: {
        // Markdown must decode identical entities in a Worker and in the UI.
        // This package's browser export touches document during module loading;
        // its public default export is the DOM-free WHATWG entity table.
        "decode-named-character-reference": createRequire(import.meta.url).resolve("decode-named-character-reference"),
        "hast-util-from-html-isomorphic": createRequire(import.meta.url).resolve("hast-util-from-html-isomorphic"),
        "node-fetch": fileURLToPath(new URL("./src/shims/citationFetch.ts", import.meta.url)),
        "sync-fetch": fileURLToPath(new URL("./src/shims/citationSyncFetch.ts", import.meta.url)),
      },
    },
    server: {
      port: 5173,
      strictPort: true,
    },
  };
});

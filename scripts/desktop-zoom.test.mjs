import {readFileSync} from "node:fs";
import {expect, it} from "vitest";

it("grants zoom only to the main WebView without enabling the wheel polyfill", () => {
  const root = new URL("../apps/desktop/src-tauri/", import.meta.url);
  const capability = JSON.parse(readFileSync(new URL("capabilities/main.json", root), "utf8"));
  const config = JSON.parse(readFileSync(new URL("tauri.conf.json", root), "utf8"));
  expect(capability.windows).toEqual(["main"]);
  expect(capability.permissions).toContain("core:webview:allow-set-webview-zoom");
  expect(config.app.windows[0].zoomHotkeysEnabled).not.toBe(true);
});

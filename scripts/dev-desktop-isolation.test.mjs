import { afterEach, beforeEach, expect, it, vi } from "vitest";

const dependencies = vi.hoisted(() => ({
  prepareDocling: vi.fn(),
  prepareCodex: vi.fn(),
  prepareSemantic: vi.fn(),
  preparePackages: vi.fn(() => ({directory: "/synthetic/runtime/plugins"})),
  probe: vi.fn(),
  spawnSync: vi.fn(() => ({status: 0})),
  spawn: vi.fn(() => ({
    once(event, callback) {
      if (event === "exit") queueMicrotask(() => callback(0, null));
    },
  })),
}));
vi.mock("./prepare-docling-runtime.mjs", () => ({ prepareDoclingRuntime: dependencies.prepareDocling }));
vi.mock("./prepare-codex-runtime.mjs", () => ({ prepareCodexRuntime: dependencies.prepareCodex }));
vi.mock("./prepare-semantic-engine.mjs", () => ({ prepareSemanticEngine: dependencies.prepareSemantic }));
vi.mock("./prepare-builtin-plugin-packages.mjs", () => ({prepareBuiltinPluginResources: dependencies.preparePackages}));
vi.mock("node:child_process", () => ({ spawn: dependencies.spawn, spawnSync: dependencies.spawnSync }));
vi.mock("node:net", () => ({
  createServer: () => {
    dependencies.probe();
    const callbacks = new Map();
    return {
      once(event, callback) { callbacks.set(event, callback); },
      listen() { queueMicrotask(() => callbacks.get("listening")()); },
      close(callback) { callback(); },
    };
  },
}));

import { runDesktopDevelopment } from "./dev-desktop.mjs";

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubEnv("RESEARCH_ASSISTANT_BUILTIN_PLUGINS", undefined);
  vi.stubEnv("RESEARCH_ASSISTANT_RUNTIME_PLUGINS", undefined);
  vi.stubEnv("RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY", undefined);
  dependencies.spawnSync.mockImplementation(command => command === process.execPath
    ? {status: 0, stdout: JSON.stringify({plugins: [], cargoFeatures: []})} : {status: 0});
});
afterEach(() => vi.unstubAllEnvs());

it("prepares Core resources before starting Tauri without an out-of-configuration helper build", async () => {
  expect(await runDesktopDevelopment(["--no-watch"])).toBe(0);
  expect(dependencies.prepareDocling).toHaveBeenCalledOnce();
  expect(dependencies.prepareCodex).toHaveBeenCalledOnce();
  expect(dependencies.prepareSemantic).toHaveBeenCalledOnce();
  expect(dependencies.spawnSync).not.toHaveBeenCalled();
  const [program, args, options] = dependencies.spawn.mock.calls[0];
  expect(program).toBe("pnpm");
  expect(args).toEqual(expect.arrayContaining(["dev", "--no-watch", "--no-dev-server"]));
  expect(options.stdio).toBe("inherit");
  expect(options.env.RESEARCH_ASSISTANT_BUILTIN_PLUGINS).toBe("none");
  expect(options.env.RESEARCH_ASSISTANT_RUNTIME_PLUGINS).toBe("1");
  expect(options.env.RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY).toBe("/synthetic/runtime/plugins");
  expect(dependencies.probe).not.toHaveBeenCalled();
  expect(dependencies.preparePackages).toHaveBeenCalledOnce();
});

it("waits for the development preparation hook before running Core", async () => {
  vi.stubEnv("RESEARCH_ASSISTANT_BUILTIN_PLUGINS", "none");
  dependencies.spawnSync.mockImplementation(command => command === process.execPath
    ? {status: 0, stdout: JSON.stringify({plugins: [], cargoFeatures: []})} : {status: 0});
  expect(await runDesktopDevelopment(["--no-watch"])).toBe(0);
  expect(dependencies.prepareDocling).toHaveBeenCalledOnce();
  expect(dependencies.spawnSync).not.toHaveBeenCalled();
  const [program, args, options] = dependencies.spawn.mock.calls[0];
  expect(program).toBe("pnpm");
  expect(args).toEqual(expect.arrayContaining([
    "dev", "--features", "development-data", "--no-default-features", "--no-watch",
  ]));
  expect(options.stdio).toBe("inherit");
  expect(JSON.parse(args.find(value => value.startsWith("{"))).build.beforeDevCommand).toEqual({
    script: "node ../../scripts/prepare-desktop-development.mjs", wait: true,
  });
});

it("retains an explicit package directory when starting a runtime Core-only selection", async () => {
  vi.stubEnv("RESEARCH_ASSISTANT_BUILTIN_PLUGINS", "none");
  vi.stubEnv("RESEARCH_ASSISTANT_RUNTIME_PLUGINS", "1");
  vi.stubEnv("RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY", "/synthetic/explicit/plugins");
  expect(await runDesktopDevelopment(["--no-watch"])).toBe(0);
  const [, args, options] = dependencies.spawn.mock.calls[0];
  expect(args).toContain("--no-dev-server");
  expect(options.env.RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY).toBe("/synthetic/explicit/plugins");
  expect(dependencies.preparePackages.mock.calls[0][0].environment.RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY)
    .toBe("/synthetic/explicit/plugins");
  expect(dependencies.probe).not.toHaveBeenCalled();
});

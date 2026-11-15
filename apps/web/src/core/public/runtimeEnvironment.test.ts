import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppRuntime } from "../runtime/appRuntime";
import { ensureRuntimeStructuredResources, getRuntimeEnvironment } from "./runtimeEnvironment";

const state = vi.hoisted(() => ({
  runtime: undefined as AppRuntime | undefined,
  startup: vi.fn(async (_runtime: AppRuntime) => ({ paperIds: [], importedPaperCount: 0, importedTemplateCount: 0 })),
}));
vi.mock("../runtime/appRuntime", () => ({ getAppRuntime: () => state.runtime }));
vi.mock("../runtime/structuredResourceStartup", () => ({ ensureStructuredResourceStartup: state.startup }));

function runtime(): AppRuntime {
  return { mode: "preview", storage: {} as Storage, gateways: {}, dispose: vi.fn() };
}
beforeEach(() => { state.startup.mockClear(); });

describe("plugin runtime adapter environment", () => {
  it("preserves storage identity without exposing the application registry or lifecycle controls", () => {
    state.runtime = runtime();
    const environment = getRuntimeEnvironment();
    expect(environment.storage).toBe(state.runtime.storage);
    expect(environment.mode).toBe("preview");
    expect(environment.hasStructuredResourceGateway).toBe(false);
    expect(environment).not.toHaveProperty("gateways");
    expect(environment).not.toHaveProperty("dispose");
  });

  it("keeps startup attached to the captured environment after another runtime is selected", async () => {
    const first = runtime(), second = runtime();
    state.runtime = first;
    const captured = getRuntimeEnvironment();
    state.runtime = second;
    expect(getRuntimeEnvironment().storage).toBe(second.storage);
    await ensureRuntimeStructuredResources(captured);
    expect(state.startup).toHaveBeenCalledExactlyOnceWith(first);
  });
});

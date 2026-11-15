// @vitest-environment happy-dom

import { beforeEach, describe, expect, it } from "vitest";

import {
  cacheLlmModels,
  readAllCachedLlmModels,
  readCachedLlmModels,
  removeCachedLlmModels,
} from "./llmModelCatalogCache";

const models = [{
  id: "gpt-test",
  displayName: "GPT Test",
  description: "Test model",
  isDefault: true,
  defaultReasoningEffort: "medium" as const,
  supportedReasoningEfforts: ["low", "medium"] as const,
}];

function installMemoryLocalStorage(): void {
  const values = new Map<string, string>();
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: {
      clear: () => values.clear(),
      getItem: (key: string) => values.get(key) ?? null,
      key: (index: number) => [...values.keys()][index] ?? null,
      get length() { return values.size; },
      removeItem: (key: string) => values.delete(key),
      setItem: (key: string, value: string) => values.set(key, value),
    } satisfies Storage,
  });
}

describe("LLM model catalog cache", () => {
  beforeEach(() => installMemoryLocalStorage());

  it("persists one provider catalog until explicitly removed", () => {
    cacheLlmModels("deepseek", models);

    expect(readCachedLlmModels("deepseek")).toEqual(models);
    expect(readAllCachedLlmModels().deepseek).toEqual(models);

    removeCachedLlmModels("deepseek");
    expect(readCachedLlmModels("deepseek")).toBeUndefined();
  });

  it("ignores malformed persisted data", () => {
    window.localStorage.setItem("research-assistant:llm-model-catalog:v1", JSON.stringify({
      schemaVersion: 1,
      providers: { openai: { models: [{ id: 7 }], refreshedAt: "invalid" } },
    }));

    expect(readAllCachedLlmModels()).toEqual({});
  });

  it("ignores the old account-agnostic ChatGPT cache and delegates caching to native", () => {
    const legacy = ["Sol", "Terra", "Luna"].map(name => ({ ...models[0],
      id: `gpt-5.6-${name.toLowerCase()}`, displayName: `GPT-5.6-${name}` }));
    window.localStorage.setItem("research-assistant:llm-model-catalog:v2", JSON.stringify({
      schemaVersion: 2, providers: { openai: { models: legacy, refreshedAt: "2026-10-09T00:00:00Z" } },
    }));
    expect(readCachedLlmModels("openai")).toBeUndefined();
    cacheLlmModels("openai", models);
    expect(readAllCachedLlmModels().openai).toBeUndefined();
  });
});

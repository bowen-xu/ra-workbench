/** @vitest-environment happy-dom */
import { afterEach, describe, expect, it, vi } from "vitest";
import { clearAgentSessionSelection, readAgentSessionSelection, writeAgentSessionSelection } from "./agentSessionSelection";

const binding = { kind: "note" as const, resourceId: "page-1" };
const values = new Map<string, string>();
const storage = {
  getItem: (key: string) => values.get(key) ?? null,
  setItem: (key: string, value: string) => { values.set(key, value); },
  removeItem: (key: string) => { values.delete(key); },
  clear: () => { values.clear(); },
} as Storage;

vi.stubGlobal("window", { localStorage: storage });

afterEach(() => window.localStorage.clear());

describe("agent session selection", () => {
  it("round trips a selection per bound host", () => {
    expect(readAgentSessionSelection(binding)).toBeNull();
    writeAgentSessionSelection(binding, "conversation-1");
    expect(readAgentSessionSelection(binding)).toBe("conversation-1");
  });

  it("does not clear a newer selection when an old deletion arrives", () => {
    writeAgentSessionSelection(binding, "conversation-2");
    clearAgentSessionSelection(binding, "conversation-1");
    expect(readAgentSessionSelection(binding)).toBe("conversation-2");
    clearAgentSessionSelection(binding, "conversation-2");
    expect(readAgentSessionSelection(binding)).toBeNull();
  });
});

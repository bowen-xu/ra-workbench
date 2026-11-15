import { describe, expect, it, vi } from "vitest";

import { CompositionGate } from "./compositionGate";

describe("CompositionGate", () => {
  it("does not persist intermediate IME composition and commits once on confirmation", () => {
    const gate = new CompositionGate();
    const commit = vi.fn();
    gate.start();
    gate.change(commit);
    gate.change(commit);
    expect(commit).not.toHaveBeenCalled();
    gate.end(commit);
    expect(commit).toHaveBeenCalledOnce();
    gate.change(commit);
    expect(commit).toHaveBeenCalledTimes(2);
  });
});

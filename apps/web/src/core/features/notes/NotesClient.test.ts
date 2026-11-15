import { describe, expect, it, vi } from "vitest";
import type { NotesGateway } from "@research-assistant/data/notes";

import { NotesClient } from "./NotesClient";

describe("NotesClient", () => {
  it("notifies all mounted entry points after a successful mutation only", async () => {
    const gateway = { rebuildNoteIndex: vi.fn(async () => undefined) } as unknown as NotesGateway;
    const client = new NotesClient(gateway);
    const reader = vi.fn();
    const workspace = vi.fn();
    client.subscribe(reader);
    client.subscribe(workspace);
    await client.mutate((value) => value.rebuildNoteIndex());
    expect(reader).toHaveBeenCalledOnce();
    expect(workspace).toHaveBeenCalledOnce();
    await expect(client.mutate(async () => { throw new Error("failed"); })).rejects.toThrow("failed");
    expect(reader).toHaveBeenCalledOnce();
  });
});

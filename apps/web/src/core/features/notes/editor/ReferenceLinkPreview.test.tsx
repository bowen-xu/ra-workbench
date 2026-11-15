// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { NotesGateway } from "@research-assistant/data/notes";
import { NotesProvider } from "../NotesClient";
import { ReferenceLinkPreview } from "./ReferenceLinkPreview";
import { noteEditorSchema } from "./noteEditorSchema";

it("uses shared links for historical references and preserves unavailable labels", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"), root = createRoot(host);
  const open = vi.fn();
  window.addEventListener("research-assistant:open-research-target", open);
  const gateway = { getReadingNote: async () => { throw new Error("not_found"); } } as unknown as NotesGateway;
  try {
    await act(async () => root.render(<NotesProvider gateway={gateway}>
      <ReferenceLinkPreview targetKind="reading_note" targetId="missing" labelSnapshot="保留标题" />
      <ReferenceLinkPreview targetKind="paper" targetId="paper" labelSnapshot="论文" />
    </NotesProvider>));
    const links = host.querySelectorAll<HTMLButtonElement>(".note-page-link-block.inline-reference");
    expect(links).toHaveLength(2);
    expect(links[0]?.disabled).toBe(true);
    expect(links[0]?.classList.contains("unavailable")).toBe(true);
    expect(links[0]?.textContent).toBe("保留标题");
    expect(host.textContent).not.toContain("目标不可用，显示插入快照");
    expect(host.querySelector(".reading-note-reference, .research-object-reference")).toBeNull();
    await act(async () => links[1]!.click());
    expect(open.mock.calls[0]?.[0].detail).toEqual({ kind: "paper", id: "paper", labelSnapshot: "论文" });
    expect(noteEditorSchema.blockSchema).not.toHaveProperty("readingNoteReference");
    expect(noteEditorSchema.blockSchema).not.toHaveProperty("researchObjectReference");
  } finally {
    await act(async () => root.unmount());
    window.removeEventListener("research-assistant:open-research-target", open);
    vi.unstubAllGlobals();
  }
});

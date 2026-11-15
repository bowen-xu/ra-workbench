// @vitest-environment happy-dom
import {expect, it, vi} from "vitest";
import {loadBrowserPlugins} from "./loadBrowserPlugins";

vi.mock("@research-assistant/literature-library-plugin/workspace", () => ({LiteratureWorkspace: () => null}));
vi.mock("@research-assistant/note-manager-plugin/workspace", () => ({NotesWorkspace: () => null}));
vi.mock("@research-assistant/writing-projects-plugin/workspace", () => ({WritingWorkspace: () => null}));

it("loads the canonical activation entries in their resolved dependency order", async () => {
  const loaded = await loadBrowserPlugins();
  expect(loaded.plugins.map(plugin => plugin.manifest.id))
    .toEqual(["literature_library", "writing_projects", "note_manager"]);
  for (const plugin of loaded.plugins) {
    expect(typeof plugin.module.activate).toBe("function");
    expect(plugin.module.configuration?.owner).toBe(plugin.manifest.id);
    expect(plugin.entryUrl).toBe(`/plugins/${plugin.manifest.id}/src/index.ts`);
    expect(plugin).not.toHaveProperty("packageKey");
  }
  expect(loaded.unavailableOptional).toEqual([]);
});

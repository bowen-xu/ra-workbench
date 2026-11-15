import {pluginDeclarationFixtures as defaultWorkbench} from "./pluginDeclarations.testSupport";
// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it} from "vitest";
import {BuiltinPluginConfiguration, BuiltinPluginConfigurations} from "./pluginProviders.testSupport";
import {usePluginConfiguration, usePluginSetting, type PluginConfiguration} from "../core/public/configuration";
import {selectAppStorage} from "../core/runtime/appStorage";
import {literatureSortSetting} from "@research-assistant/literature-library-plugin/configuration";
import {noteExplorerSetting, noteFullIndexSetting, noteModeSetting, noteScopeSetting} from "@research-assistant/note-manager-plugin/configuration";
import {writingOutlineSetting} from "@research-assistant/writing-projects-plugin/configuration";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;

it("binds all three real declarations to existing preference keys without granting another owner's keys", async () => {
  localStorage.clear(); selectAppStorage(localStorage);
  const prefix = "research-assistant:workspace.v1:";
  localStorage.setItem(prefix + "library-sort", JSON.stringify({key: "title", direction: "ascending"}));
  localStorage.setItem(prefix + "notes-mode", JSON.stringify("dependent"));
  localStorage.setItem(prefix + "notes-scope", JSON.stringify("archive"));
  localStorage.setItem(prefix + "writing-outline:one", "false");
  const bindings = new Map<string, PluginConfiguration>();
  function Literature() {
    const configuration = usePluginConfiguration(); bindings.set("literature", configuration);
    const [sort, update] = usePluginSetting(literatureSortSetting);
    return <button data-owner="literature" onClick={() => update({key: "year", direction: "descending"})}>{sort.key}</button>;
  }
  function Notes() {
    const configuration = usePluginConfiguration(); bindings.set("notes", configuration);
    const [mode] = usePluginSetting(noteModeSetting), [scope] = usePluginSetting(noteScopeSetting);
    const [explorer, toggle] = usePluginSetting(noteExplorerSetting), [fullIndex] = usePluginSetting(noteFullIndexSetting);
    return <button data-owner="notes" onClick={() => toggle(previous => !previous)}>{`${mode}/${scope}/${explorer}/${fullIndex}`}</button>;
  }
  function Writing({resourceId}: {readonly resourceId: string}) {
    const configuration = usePluginConfiguration(); bindings.set("writing", configuration);
    const [visible, toggle] = usePluginSetting(writingOutlineSetting, {resourceId});
    return <button data-owner="writing" onClick={() => toggle(previous => !previous)}>{String(visible)}</button>;
  }
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  const render = (resourceId = "one", key = "initial") => <BuiltinPluginConfigurations key={key} selected={defaultWorkbench.plugins} registrations={defaultWorkbench.configurations}>
    <BuiltinPluginConfiguration owner="literature_library"><Literature/></BuiltinPluginConfiguration>
    <BuiltinPluginConfiguration owner="note_manager"><Notes/></BuiltinPluginConfiguration>
    <BuiltinPluginConfiguration owner="writing_projects"><Writing resourceId={resourceId}/></BuiltinPluginConfiguration>
  </BuiltinPluginConfigurations>;
  const button = (owner: string) => host.querySelector<HTMLButtonElement>(`[data-owner="${owner}"]`);
  try {
    await act(async () => root.render(render()));
    expect(button("literature")?.textContent).toBe("title");
    expect(button("notes")?.textContent).toBe("dependent/archive/false/false");
    expect(button("writing")?.textContent).toBe("false");
    expect(() => bindings.get("literature")?.update("mode", "all")).toThrow("configuration_not_authorized");
    expect(() => bindings.get("notes")?.update("mode", "invalid")).toThrow("configuration_value_invalid");
    await act(async () => { button("literature")?.click(); button("notes")?.click(); });
    expect(JSON.parse(localStorage.getItem(prefix + "library-sort") ?? "null")).toEqual({key: "year", direction: "descending"});
    expect(localStorage.getItem(prefix + "notes-explorer-open")).toBe("true");
    await act(async () => root.render(render("two")));
    expect(button("writing")?.textContent).toBe("true");
    expect(localStorage.getItem(prefix + "writing-outline:two")).toBeNull();
    await act(async () => button("writing")?.click());
    expect(localStorage.getItem(prefix + "writing-outline:two")).toBe("false");
    await act(async () => root.render(render("two", "reopen")));
    expect(button("literature")?.textContent).toBe("year");
    expect(button("notes")?.textContent).toBe("dependent/archive/true/false");
    expect(button("writing")?.textContent).toBe("false");
    expect(localStorage.length).toBe(6);
  } finally { await act(async () => root.unmount()); host.remove(); }
});

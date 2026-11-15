import {decodePluginManifest, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import {resolveBuiltinPlugins} from "@research-assistant/workbench-sdk/composition";
import {literatureLibraryManifest} from "@research-assistant/literature-library-plugin/manifest";
import {noteManagerManifest} from "@research-assistant/note-manager-plugin/manifest";
import {writingProjectsManifest} from "@research-assistant/writing-projects-plugin/manifest";
import type {RuntimePluginModule} from "@research-assistant/workbench-sdk/activation";
import type {RuntimePluginLoadResult} from "../../../core/runtime/plugins/loader";
import {coreCapabilities} from "../../coreCapabilities";

/** Browser previews import the real plugin activation entries and retain their
 * injected gateways. Desktop discovery continues to use verified package URLs. */
export async function loadBrowserPlugins(): Promise<RuntimePluginLoadResult> {
  const catalog = [
    {declaration: literatureLibraryManifest, load: () => import("@research-assistant/literature-library-plugin")},
    {declaration: writingProjectsManifest, load: () => import("@research-assistant/writing-projects-plugin")},
    {declaration: noteManagerManifest, load: () => import("@research-assistant/note-manager-plugin")},
  ];
  const manifests = catalog.map(({declaration}) => decodePluginManifest({...declaration,
    formatVersion: 1, version: "1.0.0", entry: "src/index.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME}));
  const resolution = resolveBuiltinPlugins(manifests, manifests.map(manifest => manifest.id), coreCapabilities);
  const plugins = await Promise.all(resolution.activationOrder.map(async manifest => {
    const source = catalog.find(item => item.declaration.id === manifest.id)!;
    const module: RuntimePluginModule = await source.load();
    return {manifest, module, entryUrl: "/plugins/" + manifest.id + "/src/index.ts", styles: []};
  }));
  return {plugins, bindings: resolution.bindings, unavailableOptional: resolution.unavailableOptional};
}

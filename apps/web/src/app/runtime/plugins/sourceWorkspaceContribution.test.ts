import {expect, it} from "vitest";
import {assembleWorkbench} from "../../workbenchComposition";
import type {RegisteredPluginView} from "../../../core/runtime/plugins/viewRegistry";
import type {RuntimePluginStatus} from "./runtimePluginWorkbench";
import {libraryWorkspaceContribution, sourceDomainProvider} from "./sourceWorkspaceContribution";

const composition = assembleWorkbench([]);
const library: RegisteredPluginView = {key: "plugin:literature_library/library", pluginId: "literature_library",
  id: "library", title: "Library", render: () => null};
const status = (pluginId: string, state: RuntimePluginStatus["state"]): RuntimePluginStatus =>
  ({pluginId, state, entryUrl: "/fixture/entry.js", version: "1.0.0"});

it("selects the original library role only from its owning package", () => {
  const foreign = {...library, key: "plugin:foreign/library", pluginId: "foreign"};
  expect(libraryWorkspaceContribution([foreign])).toBeUndefined();
  expect(sourceDomainProvider(composition, [foreign], [status("foreign", "ready")], "literature_library")).toBeUndefined();
  expect(libraryWorkspaceContribution([foreign, library])).toBe(library);
  expect(sourceDomainProvider(composition, [library], [status("literature_library", "ready")], "literature_library")).toBe(library);
});

it.each(["discovered", "activating", "failed", "deactivating", "disabled"] as const)("does not admit source requests while the registered owner is %s", state => {
  expect(sourceDomainProvider(composition, [library], [status("literature_library", state)], "literature_library")).toBeUndefined();
});


it("admits runtime writing sources only after the exact file owner binding is ready", () => {
  const writing: RegisteredPluginView = {key: "plugin:writing_projects/writing", pluginId: "writing_projects",
    id: "writing", title: "Writing", render: () => null, writing: {Runtime: () => null, Session: () => null}};
  const ready = [status("writing_projects", "ready")];
  expect(sourceDomainProvider(composition, [writing], ready, "writing_projects")).toBeUndefined();
  expect(sourceDomainProvider(composition, [writing], ready, "writing_projects", {...writing})).toBeUndefined();
  expect(sourceDomainProvider(composition, [writing], ready, "writing_projects", writing)).toBe(writing);
  expect(sourceDomainProvider(composition, [writing], [status("writing_projects", "failed")], "writing_projects", writing)).toBeUndefined();
});

import {literatureLibraryManifest} from "@research-assistant/literature-library-plugin/manifest";
import {noteManagerManifest} from "@research-assistant/note-manager-plugin/manifest";
import {writingProjectsManifest} from "@research-assistant/writing-projects-plugin/manifest";
import {literatureConfiguration} from "@research-assistant/literature-library-plugin/configuration";
import {noteConfiguration} from "@research-assistant/note-manager-plugin/configuration";
import {writingConfiguration} from "@research-assistant/writing-projects-plugin/configuration";
import {resolveBuiltinPlugins} from "@research-assistant/workbench-sdk/composition";
import {coreCapabilities} from "./coreCapabilities";
import type {ConfigurationRegistration} from "../core/configuration/configuration";

/** Pure declaration fixtures for provider and individual component tests.
 * No view, workspace module, static plugin loader or App composition is supplied. */
export const pluginManifestFixtures = [literatureLibraryManifest, noteManagerManifest, writingProjectsManifest];
const resolution = resolveBuiltinPlugins(pluginManifestFixtures, pluginManifestFixtures.map(plugin => plugin.id), coreCapabilities);
const prefix = "research-assistant:workspace.v1:";
const configurations: readonly ConfigurationRegistration[] = [
  {owner: "literature_library", declaration: literatureConfiguration, storage: {sort: {application: prefix + "library-sort"}}},
  {owner: "note_manager", declaration: noteConfiguration, storage: {
    mode: {application: prefix + "notes-mode"}, scope: {application: prefix + "notes-scope"},
    explorerOpen: {application: prefix + "notes-explorer-open"}, fullIndex: {application: prefix + "notes-full-index"},
  }},
  {owner: "writing_projects", declaration: writingConfiguration, storage: {outlineVisible: {
    application: prefix + "writing-outline", resourcePrefix: prefix + "writing-outline:",
  }}},
];
export const pluginDeclarationFixtures = {plugins: pluginManifestFixtures, bindings: resolution.bindings, configurations};

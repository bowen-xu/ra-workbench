import type {PluginConfiguration} from "../../../core/contracts/configuration";
import {ConfigurationError, createPluginConfigurations, type ConfigurationRegistration} from "../../../core/configuration/configuration";
import type {LoadedRuntimePlugin} from "../../../core/runtime/plugins/loader";

const workspacePrefix = "research-assistant:workspace.v1:";
const legacySettings: Readonly<Record<string, ConfigurationRegistration["storage"]>> = {
  note_manager: {
    mode: {application: workspacePrefix + "notes-mode"},
    scope: {application: workspacePrefix + "notes-scope"},
    explorerOpen: {application: workspacePrefix + "notes-explorer-open"},
    fullIndex: {application: workspacePrefix + "notes-full-index"},
  },
  literature_library: {sort: {application: workspacePrefix + "library-sort"}},
  writing_projects: {outlineVisible: {
    application: workspacePrefix + "writing-outline", resourcePrefix: workspacePrefix + "writing-outline:",
  }},
};

/** App chooses storage addresses. A module contributes only its pure schema,
 * never a Storage handle or a key belonging to another owner. */
export function bindRuntimePluginConfigurations(storage: Storage, plugins: readonly LoadedRuntimePlugin[]): ReadonlyMap<string, PluginConfiguration> {
  const registrations: ConfigurationRegistration[] = [];
  for (const plugin of plugins) {
    const declaration = plugin.module.configuration;
    if (declaration === undefined) continue;
    const owner = plugin.manifest.id;
    // The imported module is JavaScript. Guard the fields needed to derive App
    // mappings, then leave schema/value/scope validation to the existing owner.
    if (!declaration || typeof declaration !== "object" || Array.isArray(declaration) || !Array.isArray(declaration.settings)) {
      throw new ConfigurationError("configuration_declaration_invalid", owner);
    }
    if (declaration.owner !== owner) throw new ConfigurationError("configuration_owner_mismatch", owner);
    for (const setting of declaration.settings) {
      if (!setting || typeof setting !== "object" || typeof setting.key !== "string" || !Array.isArray(setting.scopes)) {
        throw new ConfigurationError("configuration_declaration_invalid", owner);
      }
    }
    registrations.push({owner, declaration, storage: Object.fromEntries(declaration.settings.map(setting => {
      const prefix = `research-assistant:runtime-plugin.v1:${owner}:${setting.key}`;
      const legacy = Object.hasOwn(legacySettings, owner) ? legacySettings[owner] : undefined;
      const mapping = legacy && Object.hasOwn(legacy, setting.key) ? legacy[setting.key] : undefined;
      return [setting.key, {
        ...(setting.scopes.includes("application") ? {application: mapping?.application ?? prefix} : {}),
        ...(setting.scopes.includes("resource") ? {resourcePrefix: mapping?.resourcePrefix ?? prefix + ":resource:"} : {}),
      }];
    }))});
  }
  return createPluginConfigurations(storage, registrations);
}

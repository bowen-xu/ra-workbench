import {createContext, useContext, useMemo, useState, type PropsWithChildren} from "react";
import {createPluginConfigurations, ConfigurationError, type ConfigurationRegistration} from "../core/configuration/configuration";
import {PluginConfigurationProvider} from "../core/configuration/ConfigurationProvider";
import type {PluginConfiguration} from "../core/contracts/configuration";
import {getAppStorage} from "../core/runtime/appStorage";
import type {AgentExtensionFacets} from "../core/contracts/agentExtensions";
import {AgentExtensionProvider} from "../core/extensions/AgentExtensionProvider";
import type {BuiltinCapabilityBinding} from "./pluginComposition";
import {bindBuiltinAgentExtensions} from "./builtinAgentExtensions";

const BoundConfigurations = createContext<ReadonlyMap<string, PluginConfiguration> | undefined>(undefined);

/** The application selects owners and storage mappings. This is not a plugin
 * entrypoint and does not create another preference persistence mechanism. */
export function BuiltinPluginConfigurations({children, selected, registrations}: PropsWithChildren<{readonly selected: readonly {readonly id: string}[]; readonly registrations: readonly ConfigurationRegistration[]}>) {
  const [configurations] = useState(() => {
    const selectedIds = new Set(selected.map(plugin => plugin.id));
    return createPluginConfigurations(getAppStorage(), registrations.filter(registration => selectedIds.has(registration.owner)));
  });
  return <BoundConfigurations.Provider value={configurations}>{children}</BoundConfigurations.Provider>;
}

export function BuiltinPluginConfiguration({owner, children}: PropsWithChildren<{readonly owner: string}>) {
  const configuration = useContext(BoundConfigurations)?.get(owner);
  if (!configuration) throw new ConfigurationError("configuration_unavailable", owner);
  return <PluginConfigurationProvider value={configuration}>{children}</PluginConfigurationProvider>;
}

const BoundExtensions = createContext<ReadonlyMap<string, AgentExtensionFacets> | undefined>(undefined);
export function BuiltinAgentExtensions({selected, bindings, children}: PropsWithChildren<{readonly selected: readonly {readonly id: string}[]; readonly bindings: readonly BuiltinCapabilityBinding[]}>) {
  const facets = useMemo(() => bindBuiltinAgentExtensions(selected, bindings), [selected, bindings]);
  return <BoundExtensions.Provider value={facets}>{children}</BoundExtensions.Provider>;
}
export function BuiltinAgentExtensionScope({owner, children}: PropsWithChildren<{readonly owner: string}>) {
  const facets = useContext(BoundExtensions)?.get(owner);
  if (!facets) throw new Error(`agent_extension_consumer_unavailable: ${owner}`);
  return <AgentExtensionProvider value={facets}>{children}</AgentExtensionProvider>;
}

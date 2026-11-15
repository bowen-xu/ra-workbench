import {pluginDeclarationFixtures as defaultWorkbench} from "./pluginDeclarations.testSupport";
import type {PropsWithChildren} from "react";
import {BuiltinPluginConfiguration, BuiltinPluginConfigurations} from "./pluginProviders.testSupport";
import {BuiltinAgentExtensions, BuiltinAgentExtensionScope} from "./pluginProviders.testSupport";

/** Exact test-only composition entry. The boundary guard admits static test
 * imports, never production imports, dynamic imports or re-exports. */
export function BuiltinPluginConfigurationTest({owner, children}: PropsWithChildren<{readonly owner: string}>) {
  return <BuiltinAgentExtensions selected={[{id: owner}]} bindings={defaultWorkbench.bindings}><BuiltinPluginConfigurations selected={[{id: owner}]} registrations={defaultWorkbench.configurations}><BuiltinAgentExtensionScope owner={owner}><BuiltinPluginConfiguration owner={owner}>{children}</BuiltinPluginConfiguration></BuiltinAgentExtensionScope></BuiltinPluginConfigurations></BuiltinAgentExtensions>;
}

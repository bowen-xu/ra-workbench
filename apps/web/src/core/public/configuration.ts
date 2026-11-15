/** App-bound settings access; registration and storage mappings remain private. */
export {usePluginConfiguration, usePluginSetting} from "../configuration/ConfigurationProvider";
export type {ConfigurationSchema, ConfigurationScope, ConfigurationSetting, ConfigurationInspection, PluginConfiguration, PluginConfigurationDeclaration} from "../contracts/configuration";

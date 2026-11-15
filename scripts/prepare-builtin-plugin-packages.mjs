import {existsSync, mkdirSync, statSync} from 'node:fs';
import path from 'node:path';

/** Product assembly supplies verified plugin packages through this existing
 * runtime directory. Core owns no plugin source or plugin build pipeline. */
export function prepareBuiltinPluginResources({root = process.cwd(), environment = process.env, pluginIds} = {}) {
  const configured = environment.RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY;
  const directory = configured === undefined ? path.join(root, '.tmp/runtime-plugins/empty') : path.resolve(configured);
  if (configured === undefined) mkdirSync(directory, {recursive: true});
  if (!existsSync(directory) || !statSync(directory).isDirectory()) throw new Error('runtime_plugin_directory_invalid');
  return {directory, plugins: pluginIds ?? [], external: configured !== undefined};
}

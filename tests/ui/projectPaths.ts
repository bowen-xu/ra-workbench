import {resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {createRequire} from "node:module";

const project = new URL("../../", import.meta.url);
const require = createRequire(new URL("package.json", project));
export const projectPath = (relative: string): string => fileURLToPath(new URL(relative, project));
export const publicPackagePath = (name: string): string => require.resolve(name);

export const referenceProjectPath = (relative: string): string => resolve(
  process.env.RA_REFERENCE_PROJECTS_DIRECTORY ?? projectPath("docs/references/projects/"), relative,
);

import path from "node:path";

// Normalize the integration checkout's physical submodule locations to the
// existing ownership contracts. This does not grant new dependency edges.
export const projectSources = [
  ["ra-workbench/apps", "apps"],
  ["ra-workbench/packages", "packages"],
  ["ra-workbench-sdk/packages", "packages"],
  ["plugins/ra-literature-library", "plugins/literature_library"],
  ["plugins/ra-note-manager", "plugins/note_manager"],
  ["plugins/ra-writing-projects", "plugins/writing_projects"],
];

export function logicalProjectPath(root, file) {
  const relative = path.relative(root, file).split(path.sep).join("/");
  const mapping = projectSources.find(([physical]) => relative === physical || relative.startsWith(physical + "/"));
  return mapping ? mapping[1] + relative.slice(mapping[0].length) : relative;
}

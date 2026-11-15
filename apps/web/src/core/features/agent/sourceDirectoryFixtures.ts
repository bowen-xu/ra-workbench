import type {SourceDirectoryNode} from "@research-assistant/core/source";

/** Synthetic catalogs used by component tests still enter through a namespace. */
export function withSourceNamespaces(catalog: SourceDirectoryNode<unknown>): SourceDirectoryNode<unknown> {
  const titles: Record<string, string> = {library: "所有文献", notes: "所有笔记", writings: "所有写作项目", agents: "Agent 资源", assets: "通用资产"};
  return {value: catalog.value, child: async root => titles[root]
    ? {value: {kind: "directory", title: titles[root]!}, child: catalog.child} : undefined};
}

import {invoke} from "../runtime/platformCore";
import {runtimeAgentGateway} from "../runtime/appRuntime";

/** Catalog identity only; selection never installs or switches a CLI binary. */
export interface CodexCatalogVersions {
  readonly runtimeVersion: string;
  readonly selectedVersion: string;
  readonly availableVersions: readonly string[];
}
export interface CodexCatalogVersionGateway {
  getVersions(refresh?: boolean): Promise<CodexCatalogVersions>;
  selectVersion(version: string): Promise<CodexCatalogVersions>;
}
export function decodeCodexCatalogVersions(value: unknown): CodexCatalogVersions | undefined {
  if (!value || typeof value !== "object") return;
  const data = value as Record<string, unknown>;
  const version = (item: unknown): item is string => typeof item === "string" && /^(0|[1-9]\d{0,8})\.(0|[1-9]\d{0,8})\.(0|[1-9]\d{0,8})$/u.test(item);
  if (!version(data.runtimeVersion) || !version(data.selectedVersion) || !Array.isArray(data.availableVersions) || data.availableVersions.length < 1 || data.availableVersions.length > 20 || !data.availableVersions.every(version) || !data.availableVersions.includes(data.selectedVersion)) return;
  return {runtimeVersion: data.runtimeVersion, selectedVersion: data.selectedVersion, availableVersions: data.availableVersions};
}
async function read(command: string, args: Record<string, unknown>): Promise<CodexCatalogVersions> {
  const result = decodeCodexCatalogVersions(await invoke<unknown>(command, args));
  if (!result) throw new Error("Codex CLI 版本信息格式无效（codex_version_response）");
  return result;
}
export const codexCatalogVersionGateway: CodexCatalogVersionGateway = runtimeAgentGateway("codexCatalogVersionGateway", {
  getVersions: (refresh = false) => read("get_codex_catalog_versions", {refresh}),
  selectVersion: (version: string) => read("select_codex_catalog_version", {version}),
});

import {assertPluginCompatibility, decodePluginManifest, hasPluginFrontend, type RuntimePluginManifest} from "@research-assistant/workbench-sdk";
import {resolveBuiltinPlugins} from "@research-assistant/workbench-sdk/composition";
import {invoke} from "../runtime/platformCore";
import {open} from "../runtime/platformDialog";
import type {ManagedPluginView, PluginManagementViewGateway} from "../features/settings/PluginManagementSettings";

interface InstalledPlugin {
  readonly packageKey: string;
  readonly manifest: unknown;
  readonly enabled: boolean;
  readonly removed: boolean;
  readonly startupPackageKey: string | null;
  readonly origin: "installed" | "bundled";
  readonly diagnostic: unknown;
}
interface Listing {readonly revision: string; readonly plugins: readonly InstalledPlugin[];}
interface RuntimeStatus {readonly pluginId: string; readonly state: string; readonly error?: Error;}
interface Effects {
  readonly invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  readonly choosePackage: () => Promise<string | null>;
}

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("plugin_management_response_invalid");
  return value as Record<string, unknown>;
}
function contentKey(value: unknown): string {
  if (typeof value !== "string" || !/^[a-f0-9]{64}$/u.test(value)) throw new Error("plugin_management_response_invalid: packageKey");
  return value;
}
function decodeListing(value: unknown): Listing {
  const raw = record(value);
  const revision = contentKey(raw.revision);
  if (!Array.isArray(raw.plugins) || raw.plugins.length > 192) throw new Error("plugin_management_response_invalid: plugins");
  const plugins = raw.plugins.map((value: unknown): InstalledPlugin => {
    const row = record(value), pkg = record(row.package);
    if (typeof row.enabled !== "boolean" || typeof row.removed !== "boolean"
      || (row.origin !== "bundled" && row.origin !== "installed")) throw new Error("plugin_management_response_invalid: state");
    return {packageKey: contentKey(pkg.packageKey), manifest: pkg.manifest, enabled: row.enabled,
      removed: row.removed, startupPackageKey: row.startupPackageKey === null ? null : contentKey(row.startupPackageKey), origin: row.origin, diagnostic: row.diagnostic};
  });
  return {revision, plugins};
}

function failure(reason: unknown): Error {
  if (reason instanceof Error) return reason;
  if (reason && typeof reason === "object") {
    const value = reason as Record<string, unknown>;
    if (typeof value.code === "string" && typeof value.message === "string") {
      return new Error(`${value.code}：${value.message}${typeof value.packageKey === "string" ? ` / ${value.packageKey}` : ""}`);
    }
  }
  return new Error(typeof reason === "string" ? reason : "plugin_management_failed");
}
const descriptions: Record<string, {name: string; description: string}> = {
  literature_library: {name: "文献管理", description: "整理文献、阅读 PDF，管理来源与研究证据。"},
  note_manager: {name: "笔记管理", description: "在工作台中组织、搜索和管理笔记。"},
  writing_projects: {name: "写作管理", description: "组织稿件、引用与编译流程。"},
};

/** Settings administration is Core-owned. SDK preflight is reused without
 * importing any plugin module. Native authority remains in its existing host. */
export function createPluginManagementGateway(options: {
  readonly coreCapabilities: readonly {readonly id: string; readonly version: number}[];
  readonly readRuntime: () => readonly RuntimeStatus[];
  readonly isInitialLoadComplete: () => boolean;
  readonly subscribe?: (listener: () => void) => () => void;
}, effects: Effects = {
  invoke,
  choosePackage: async () => {
    const path = await open({multiple: false, title: "选择插件包 manifest.json 或 ZIP", filters: [{name: "本地插件包", extensions: ["json", "zip"]}]});
    return typeof path === "string" ? path : null;
  },
}): PluginManagementViewGateway {
  let listing: Listing | undefined;
  async function call(command: string, args?: Record<string, unknown>): Promise<unknown> {
    try {return await effects.invoke(command, args);}
    catch (reason) {throw failure(reason);}
  }
  function view(row: InstalledPlugin): ManagedPluginView {
    const raw = record(row.manifest);
    const id = typeof raw.id === "string" ? raw.id : row.packageKey;
    let manifest: RuntimePluginManifest | undefined;
    let diagnostic: string | undefined;
    try {
      manifest = decodePluginManifest(raw);
      assertPluginCompatibility(manifest);
      if (row.enabled && !row.removed) preflight(listing?.plugins ?? []);
    } catch (reason) {diagnostic = failure(reason).message;}
    if (row.diagnostic) diagnostic = failure(row.diagnostic).message;
    const status = options.readRuntime().find(status => status.pluginId === id);
    if (!diagnostic && status?.error) diagnostic = status.error.message;
    const pending = row.removed || row.startupPackageKey !== (row.enabled ? row.packageKey : null);
    if (!pending && !diagnostic && row.enabled && manifest && hasPluginFrontend(manifest)
      && status?.state !== "ready" && options.isInitialLoadComplete()) {
      diagnostic = "plugin_not_active：插件未在本次运行中激活，请核对启动诊断后重启工作台。";
    }
    const runtime: ManagedPluginView["runtime"] = pending ? "restart" : diagnostic ? "blocked" : !row.enabled ? "disabled"
      : status?.state === "ready" || (manifest && !hasPluginFrontend(manifest)) ? "active" : "loading";
    return {
      id, name: descriptions[id]?.name ?? id,
      description: descriptions[id]?.description ?? "独立安装的本地工作台插件。",
      version: typeof raw.version === "string" ? raw.version : "未知",
      enabled: row.enabled, runtime, pendingRemoval: row.removed, origin: row.origin,
      dependencies: manifest ? [...manifest.dependencies.required.map(dep => `${dep.id} · API ${dep.apiVersion}`),
        ...manifest.dependencies.optional.map(dep => `${dep.id} · API ${dep.apiVersion}（可选）`)] : [],
      capabilities: manifest?.provides.map(capability => `${capability.id} · v${capability.version}`) ?? [],
      ...(diagnostic ? {diagnostic} : {}),
    };
  }
  function preflight(rows: readonly InstalledPlugin[]): void {
    const enabled = rows.filter(row => row.enabled && !row.removed).map(row => decodePluginManifest(row.manifest));
    for (const manifest of enabled) assertPluginCompatibility(manifest);
    resolveBuiltinPlugins(enabled, enabled.map(manifest => manifest.id), options.coreCapabilities);
  }
  function current(): Listing {
    if (!listing) throw new Error("plugin_management_refresh_required：请先刷新插件列表。");
    return listing;
  }
  async function list(): Promise<readonly ManagedPluginView[]> {
    listing = decodeListing(await call("list_managed_plugins"));
    return listing.plugins.map(view);
  }
  async function mutate(id: string, enabled: boolean, removed: boolean): Promise<void> {
    const snapshot = current();
    const row = snapshot.plugins.find(row => record(row.manifest).id === id);
    if (!row || row.removed) throw new Error("plugin_not_installed：插件尚未安装或已安排卸载。");
    if (enabled && row.diagnostic) throw failure(row.diagnostic);
    const proposed = snapshot.plugins.map(item => item === row ? {...item, enabled, removed} : item);
    preflight(proposed);
    await call(removed ? "uninstall_managed_plugin" : "set_managed_plugin_enabled", {
      pluginId: id, ...(removed ? {} : {enabled}), expectedRevision: snapshot.revision,
    });
  }
  return {
    list,
    ...(options.subscribe ? {subscribe: options.subscribe} : {}),
    async importPackage() {
      const path = await effects.choosePackage();
      if (path === null) return null;
      const installed = record(await call("install_managed_plugin", {source: path, expectedRevision: current().revision}));
      const key = contentKey(installed.packageKey);
      const rows = await list();
      const id = record(installed.manifest).id;
      const result = rows.find(row => row.id === id);
      if (!result) throw new Error(`plugin_install_receipt_missing: ${key}`);
      return result;
    },
    setEnabled: (id, enabled) => mutate(id, enabled, false),
    uninstall: id => mutate(id, false, true),
  };
}

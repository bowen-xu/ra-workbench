import {createRoot} from "react-dom/client";
import {isTauri, invoke} from "@tauri-apps/api/core";
import {composition} from "virtual:workbench-composition";
import {WorkbenchApp} from "./app/WorkbenchApp";
import {ProductionWorkbenchRoot} from "./app/ProductionWorkbenchRoot";
import {selectAppRuntime, type AppRuntime} from "./core/runtime/appRuntime";
import {AppRuntimeProvider} from "./core/runtime/AppRuntimeProvider";
import {StructuredResourceStartupGate} from "./core/runtime/StructuredResourceStartupGate";
import {DeveloperModeProvider} from "./core/features/settings/DeveloperMode";
import {ApplicationScrollbars} from "./core/components/scroll/ApplicationScrollbars";
import {notify} from "./core/services/notifications";
import {coreCapabilities} from "./app/coreCapabilities";
import {createRuntimePluginWorkbench} from "./app/runtime/plugins/runtimePluginWorkbench";
import {getAppStorage} from "./core/runtime/appStorage";
import "./core/styles.css";

async function bootstrap() {
  const target = document.getElementById("root");
  if (!target) throw new Error("Root element was not found");
  if (import.meta.env.DEV && !isTauri() && new URLSearchParams(location.search).get("preview") === "plugin-settings") {
    const {mountPluginSettingsPreview} = await import("./app/previews/settings/PluginSettingsPreview");
    mountPluginSettingsPreview(target);
    return;
  }
  let runtimeDisposed = false;
  let runtimePluginLoading: {dispose(): void} | undefined;
  const runtime: AppRuntime = {mode: isTauri() ? "desktop" : "browser", storage: localStorage, gateways: {}, dispose() {
    runtimeDisposed = true;
    runtimePluginLoading?.dispose();
  }};
  selectAppRuntime(runtime); composition.configureRuntime(runtime);
  const runtimePlugins = __RUNTIME_PLUGINS__ && isTauri()
    ? createRuntimePluginWorkbench({storage: getAppStorage()}) : undefined;
  addEventListener("pagehide", () => runtime.dispose(), {once: true});
  addEventListener("pageshow", event => {if (event.persisted) location.reload();});
  createRoot(target).render(<><ApplicationScrollbars/><AppRuntimeProvider runtime={runtime}>
    <StructuredResourceStartupGate runtime={runtime}><DeveloperModeProvider><ProductionWorkbenchRoot>
      <WorkbenchApp composition={composition} {...(runtimePlugins ? {runtimePlugins} : {})}/>
    </ProductionWorkbenchRoot></DeveloperModeProvider></StructuredResourceStartupGate>
  </AppRuntimeProvider></>);
  if (__RUNTIME_PLUGINS__ && isTauri()) {
    const report = (reason: unknown) => {
      if (runtimeDisposed) return;
      const error = reason instanceof Error ? reason : new Error(String(reason));
      const cause = error.cause;
      const diagnostic = "diagnostic" in error && error.diagnostic && typeof error.diagnostic === "object"
        ? error.diagnostic : undefined;
      const version = (key: string): string | number | undefined => {
        const value: unknown = diagnostic && key in diagnostic ? Reflect.get(diagnostic, key) : undefined;
        return typeof value === "string" || typeof value === "number" ? value : undefined;
      };
      const expected = version("expectedVersion") ?? version("expected");
      const actual = version("actualVersion") ?? version("actual");
      const compatibility = expected === undefined && actual === undefined ? ""
        : `（需要 ${expected ?? "未提供"}，实际 ${actual ?? "未提供"}）`;
      const native = cause && typeof cause === "object" && "code" in cause && "message" in cause
        && typeof cause.code === "string" && /^runtime_plugin_[a-z_]+$/u.test(cause.code)
        && typeof cause.message === "string" ? ` / ${cause.code}: ${cause.message}` : "";
      const activation = cause instanceof Error && "code" in cause && typeof cause.code === "string"
        && /^(?:configuration|plugin)_[a-z_]+$/u.test(cause.code) ? ` / ${cause.message}` : "";
      notify.error(error.message + compatibility + native + activation);
    };
    try {
      const [{createRuntimePluginLoader}, {installRuntimePluginLoading}] = await Promise.all([
        import("./core/runtime/plugins/loader"), import("./app/runtime/plugins/installRuntimePluginLoading"),
      ]);
      if (runtimeDisposed) return;
      const loading = installRuntimePluginLoading({
        loader: createRuntimePluginLoader({gateway: {discover: () => invoke("list_runtime_plugin_packages")}, coreCapabilities}),
        onLoaded: result => runtimePlugins?.activate(result),
        onDeactivate: pluginId => runtimePlugins?.deactivate(pluginId)
          ?? Promise.reject(new Error(`plugin_workbench_unavailable: ${pluginId}`)),
        retainPluginStyles: entryUrl => runtimePlugins?.retainPluginStyles(entryUrl) ?? false,
        onError: report,
      });
      runtimePluginLoading = loading;
      // Package discovery/import errors must not replace the running Core UI.
      void loading.load().catch(report).finally(() => runtimePlugins?.completeInitialLoad());
    } catch (reason) { runtimePlugins?.completeInitialLoad(); report(reason); }
  }
}
void bootstrap().catch(error => {const target = document.getElementById("root"); if (target) target.textContent = error instanceof Error ? error.message : "应用启动失败";});

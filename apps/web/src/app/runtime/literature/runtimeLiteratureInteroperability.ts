import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import type {LiteratureInteroperability} from "../../../core/public/domainCapabilities";
import {getAppRuntime} from "../../../core/runtime/appRuntime";

/** Bind the four existing cross-domain methods to the same activating package.
 * Its private gateway and Core's explicit static/mock owners retain their roles. */
export function bindRuntimeLiteratureInteroperability(port: RuntimeLiteraturePort,
  available: () => boolean, packageKey: string): (() => void) | undefined {
  const runtime = getAppRuntime();
  if (runtime.mode === "browser" || runtime.mode === "preview" && !runtime.nativeAgentPreview
    || runtime.gateways.getLiteratureGateway || runtime.gateways.getLiteratureInteroperability) return undefined;
  function current(): void {
    if (getAppRuntime() !== runtime || runtime.gateways.getLiteratureInteroperability !== resolve || !available())
      throw Object.assign(new Error("literature_citations_unavailable：文献插件未就绪或原协作实例已结束。"), {
        code: "literature_citations_unavailable", pluginId: "literature_library", packageKey,
      });
  }
  const selected = Object.freeze({
    async get(id) {
      current();
      return port.literature.get(id);
    },
    async list(query) {
      current();
      return port.literature.list(query);
    },
    async save(draft) {
      current();
      return port.literature.save(draft);
    },
    async updateMetadataField(itemId, patch) {
      current();
      return port.literature.updateMetadataField(itemId, patch);
    },
  } satisfies LiteratureInteroperability);
  function resolve(): LiteratureInteroperability | undefined {
    return getAppRuntime() === runtime && runtime.gateways.getLiteratureInteroperability === resolve && available() ? selected : undefined;
  }
  runtime.gateways.getLiteratureInteroperability = resolve;
  return () => {
    if (runtime.gateways.getLiteratureInteroperability === resolve) delete runtime.gateways.getLiteratureInteroperability;
  };
}

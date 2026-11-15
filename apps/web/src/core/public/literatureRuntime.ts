import {getAppRuntime} from "../runtime/appRuntime";
import type {AppGateways} from "../runtime/appGatewayTypes";
import type {LiteratureGateway, ResearchKnowledgeGateway, EvidenceGateway, EvidenceMatrixGateway,
  DerivedResearchViewGateway, ReferenceRecordGateway, ResearchExportGateway} from "@research-assistant/data/literature";
import type {CollectionGateway} from "@research-assistant/data/collections";
import type {NalGateway, NalQueryGateway} from "@research-assistant/data/nal";


type LiteratureKey = "getLiteratureGateway" | "getCollectionGateway" | "getNalGateway" | "getNalQueryGateway"
  | "getResearchKnowledgeGateway" | "getEvidenceGateway" | "getEvidenceMatrixGateway"
  | "getDerivedResearchViewGateway" | "getReferenceRecordGateway";

/** Read one fixed domain capability selected by App. Neither runtime state nor
 * arbitrary gateway names or platform execution are exposed to consumers. */
function resolve<K extends LiteratureKey>(name: K, browserFallback: AppGateways[K]): AppGateways[K] {
  const runtime = getAppRuntime(), selected = runtime.gateways[name];
  if (selected) return selected;
  // Native Agent preview never implicitly grants literature capabilities.
  if (runtime.mode === "preview") throw new Error(`预览未覆盖业务网关：${name}`);
  if (runtime.mode === "desktop") throw Object.assign(
    new Error(`literature_gateway_unavailable：当前工作台尚未接入 ${name}。`), {code: "literature_gateway_unavailable"});
  return browserFallback;
}

export const resolveLiteratureRuntimeGateway = (fallback: LiteratureGateway): LiteratureGateway => resolve("getLiteratureGateway", fallback);
export const resolveCollectionRuntimeGateway = (fallback: CollectionGateway): CollectionGateway => resolve("getCollectionGateway", fallback);
export const resolveNalRuntimeGateway = (fallback: NalGateway): NalGateway => resolve("getNalGateway", fallback);
export const resolveNalQueryRuntimeGateway = (fallback: NalQueryGateway): NalQueryGateway => resolve("getNalQueryGateway", fallback);
export const resolveKnowledgeRuntimeGateway = (fallback: ResearchKnowledgeGateway): ResearchKnowledgeGateway => resolve("getResearchKnowledgeGateway", fallback);
export const resolveEvidenceRuntimeGateway = (fallback: EvidenceGateway): EvidenceGateway => resolve("getEvidenceGateway", fallback);
export const resolveEvidenceMatrixRuntimeGateway = (fallback: EvidenceMatrixGateway): EvidenceMatrixGateway => resolve("getEvidenceMatrixGateway", fallback);
export const resolveDerivedViewRuntimeGateway = (fallback: DerivedResearchViewGateway): DerivedResearchViewGateway => resolve("getDerivedResearchViewGateway", fallback);
export const resolveReferenceRecordRuntimeGateway = (fallback: ReferenceRecordGateway): ReferenceRecordGateway => resolve("getReferenceRecordGateway", fallback);

/** Export keeps its existing desktop/full-preview/native-preview transport
 * behavior. Browser-only execution continues to use the domain refusal port. */
export function resolveResearchExportRuntimeGateway(browserFallback: ResearchExportGateway): ResearchExportGateway {
  const runtime = getAppRuntime();
  if (runtime.mode === "browser") return browserFallback;
  const gateway = runtime.gateways.getResearchExportGateway;
  if (gateway) return gateway;
  throw Object.assign(new Error("research_export_gateway_unavailable：当前工作台尚未接入研究导出适配。"),
    {code: "research_export_gateway_unavailable"});
}

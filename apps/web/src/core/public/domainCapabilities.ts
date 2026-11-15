import type {WritingNoteCatalogGateway} from "@research-assistant/data/notes";
import type {LiteratureGateway} from "@research-assistant/data/literature";
import {getAppRuntime} from "../runtime/appRuntime";

/** Existing optional cross-domain ports selected by App. Absence is explicit;
 * querying availability neither initializes an owner nor grants tool authority. */
export type LiteratureInteroperability = Pick<LiteratureGateway, "get" | "list" | "save" | "updateMetadataField">;
export function getOptionalLiteratureGateway(): LiteratureInteroperability | undefined {
  const gateways = getAppRuntime().gateways;
  return gateways.getLiteratureInteroperability?.() ?? gateways.getLiteratureGateway;
}
export function getOptionalWritingNoteCatalog(): WritingNoteCatalogGateway | undefined {
  return getAppRuntime().gateways.getWritingGateway?.noteCatalog;
}

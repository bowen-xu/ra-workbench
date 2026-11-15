import type { LiteratureItem } from "@research-assistant/core/literature";
import type { Manuscript, ManuscriptFile } from "@research-assistant/writing";
import { getAppRuntime } from "../runtime/appRuntime";
import { getOptionalLiteratureGateway } from "../public/domainCapabilities";

/** Domain access is supplied by application assembly through the existing runtime.
 * Missing plugins do not prevent unrelated Core documents from opening.
 */
export interface LiteratureSourceAccess {
  get(id: string): Promise<LiteratureItem>;
  list(): Promise<readonly LiteratureItem[]>;
}
export interface WritingSourceAccess {
  listManuscripts(): Promise<readonly Manuscript[]>;
  listFiles(id: string): Promise<readonly ManuscriptFile[]>;
}
const missing = (name: string): Error => Object.assign(new Error(`source_domain_unavailable：${name}尚未接入当前工作台。`), {code:"source_domain_unavailable"});
const literature: LiteratureSourceAccess = {
  get: id => getOptionalLiteratureGateway()?.get(id) ?? Promise.reject(missing("文献能力")),
  list: () => getOptionalLiteratureGateway()?.list() ?? Promise.resolve([]),
};
const writing: WritingSourceAccess = {
  listManuscripts: () => getAppRuntime().gateways.getWritingGateway?.listManuscripts() ?? Promise.resolve([]),
  listFiles: id => getAppRuntime().gateways.getWritingGateway?.listFiles(id) ?? Promise.reject(missing("写作能力")),
};
export function getLiteratureGateway(): LiteratureSourceAccess { return literature; }
export function getWritingGateway(): WritingSourceAccess { return writing; }

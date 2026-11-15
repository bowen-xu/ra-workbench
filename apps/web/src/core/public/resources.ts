/** Typed content-storage service; never exposes filesystem handles or raw SQL. */
export { getContentStorageGateway } from "../services/contentStorageGateway";
export type { ContentStorageGateway, ContentStorageReport, ContentGcRunReport } from "@research-assistant/data";

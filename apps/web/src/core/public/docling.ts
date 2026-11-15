import { getDoclingGateway } from "../services/doclingGateway";
import type { DoclingParserGateway } from "../contracts/docling";

/** Use the existing runtime-selected adapter and its task/recovery path. */
export function getDoclingParserGateway(): DoclingParserGateway {
  return getDoclingGateway();
}

export type {
  DoclingParserGateway, DoclingParseTaskStatus, DoclingParseRunSummary,
  DoclingParseRunDetail, DoclingOverlayRect, DoclingOverlayRegion,
  DoclingOverlayItem, DoclingPageOverlay,
} from "../contracts/docling";
export { doclingErrorMessage } from "../services/doclingError";
export { runDoclingParse } from "../services/doclingParseTask";

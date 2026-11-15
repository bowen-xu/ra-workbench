import { resolveAppGateway } from "../../../runtime/appRuntime";
import { isDesktopRuntime } from "../../../runtime/platformCore";
import type { NotesGateway } from "@research-assistant/data/notes";

import { browserNotesGateway } from "./browserNotesGateway";
import { tauriNotesGateway } from "./tauriNotesGateway";

export function getNotesGateway(): NotesGateway {
  return resolveAppGateway("getNotesGateway", isDesktopRuntime() ? tauriNotesGateway : browserNotesGateway);
}

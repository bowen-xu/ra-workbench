import {systemDocumentPath, systemDocumentReference, type SystemDocument} from "@research-assistant/core/agent";
import {copyComposerClipboard} from "../agent/agentComposerClipboard";

/** The OS HTML carrier preserves identity; plain-text fallback remains the id-path. */
export function copySystemDocument(document: SystemDocument): Promise<boolean> {
  const path = systemDocumentPath(document.id) ?? document.path;
  return copyComposerClipboard({text:path, bindings:[{
    startUtf16:0, endUtf16:path.length, input:systemDocumentReference(document),
  }]});
}

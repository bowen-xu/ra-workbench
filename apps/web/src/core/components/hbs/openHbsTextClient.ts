import { HbsClient, type HbsClientOptions } from "@research-assistant/core/hbs";
import { createCodeMirrorHbsPolicy } from "./codeMirrorHbsPolicy";

/** Web composition root; algorithms and request identities stay outside core. */
export function openHbsTextClient(options: Omit<HbsClientOptions, "policy" | "requestId"> & Partial<Pick<HbsClientOptions, "policy" | "requestId">>): Promise<HbsClient> {
  const session = crypto.randomUUID();
  return HbsClient.open({ ...options, policy: options.policy ?? createCodeMirrorHbsPolicy(), requestId: options.requestId ?? (sequence => `${session}:${sequence}`) });
}

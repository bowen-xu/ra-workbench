import {getAppRuntime} from "../../runtime/appRuntime";
import {captureNewPreviewOwnerIntent} from "../../runtime/nativePreviewOwnerIntent";

/** Freeze mock transport ownership at operation creation, never on recovery. */
export function captureQueueOwnerIntent(scope: {conversationId: string}): ((requestId: string) => Promise<void>) | undefined {
  const runtime = getAppRuntime();
  return captureNewPreviewOwnerIntent(runtime.nativeAgentPreview, runtime.storage, scope.conversationId);
}

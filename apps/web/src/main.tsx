import {ProductionWorkbenchRoot} from "./app/ProductionWorkbenchRoot";
import { createAppRuntime, isAppPreviewRequest } from "./app/runtime/createAppRuntime";
import { AppRuntimeProvider } from "./core/runtime/AppRuntimeProvider";
import { ApplicationKeyboardPolicy } from "./core/runtime/ApplicationKeyboardPolicy";
import { StructuredResourceStartupGate } from "./core/runtime/StructuredResourceStartupGate";
import { SpeechInputAdapterProvider } from "./core/features/speech/SpeechInputAdapter";
import { writingDemoSpeechAdapter } from "./app/previews/agent/WritingDemoSpeechAdapter";
import { StrictMode, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { ApplicationScrollbars } from "./core/components/scroll/ApplicationScrollbars";

import { DeveloperModeProvider } from "./core/features/settings/DeveloperMode";
import { App } from "./app/App";
import { AgentRuntimeCanvasPage } from "./app/previews/agent/AgentRuntimeCanvasPage";
import { WritingEditorDiffDemo } from "./app/previews/agent/WritingEditorDiffDemo";
import { ConversationDeleteDemo } from "./app/previews/agent/ConversationDeleteDemo";
import { ShimmerTextDemo } from "./app/previews/components/ShimmerTextDemo";
import "./core/styles.css";
import { isSourceNavigationPreview } from "./core/runtime/sourceNavigationPreview";
import { readWritingRecoveryScenario } from "./core/runtime/writingRecoveryPreview";

const rootElement = document.getElementById("root");

if (rootElement === null) {
  throw new Error("Root element was not found");
}

const root = createRoot(rootElement);
function renderApplication(children: ReactNode): void {
  root.render(<><ApplicationScrollbars />{children}</>);
}

const fullPreview = isAppPreviewRequest(window.location.search);
const isNotesPreview = new URLSearchParams(window.location.search).get("preview") === "notes";
const isTextPatchPreview = isNotesPreview || new URLSearchParams(window.location.search).get("demo") === "text-patch";
const isAgentCanvasDemo = !fullPreview && new URLSearchParams(window.location.search).get("demo") === "agent-canvas";
const isWritingEditorDiffDemo = !fullPreview && new URLSearchParams(window.location.search).get("demo") === "writing-editor-diff";
const isConversationDeleteDemo = !fullPreview && new URLSearchParams(window.location.search).get("demo") === "conversation-delete";
const isShimmerTextDemo = !fullPreview && new URLSearchParams(window.location.search).get("demo") === "shimmer-text";

async function bootstrap(): Promise<void> {
  if (!fullPreview && new URLSearchParams(window.location.search).get("demo") === "nal-editor") {
    const { NalEditorPreview } = await import("./app/previews/components/NalEditorPreview");
    renderApplication(<StrictMode><NalEditorPreview /></StrictMode>);
    return;
  }
  if (new URLSearchParams(window.location.search).get("demo") === "reorder") {
    const { ReorderExample } = await import("../../../docs/ui-templates/reorder-template/Example");
    renderApplication(<StrictMode><ApplicationKeyboardPolicy /><ReorderExample /></StrictMode>);
    return;
  }
  const runtime = await createAppRuntime(window.location.search, window.localStorage);
  window.addEventListener("pagehide", () => runtime.dispose(), { once: true });
  window.addEventListener("pageshow", event => { if (event.persisted) window.location.reload(); });
  const app = <DeveloperModeProvider>{isShimmerTextDemo ? <ShimmerTextDemo /> : isAgentCanvasDemo ? <AgentRuntimeCanvasPage /> : isWritingEditorDiffDemo ? <WritingEditorDiffDemo /> : isConversationDeleteDemo ? <ConversationDeleteDemo /> :
  <ProductionWorkbenchRoot>
      <App
        initialChatOpen={runtime.mode === "preview" && !isNotesPreview && (isTextPatchPreview || isSourceNavigationPreview(window.location.search))}
        {...(isTextPatchPreview ? { initialLocation: { view: "notes" as const, notePageId: "00000000-0000-4000-8000-000000000022" } } : {})}
        {...(runtime.mode === "preview" && readWritingRecoveryScenario(window.location.search) ? { initialLocation: { view: "writing" as const } } : {})}
        {...(runtime.noteRecovery ? {initialLocation: {view: "notes" as const, notePageId: runtime.noteRecovery.pageId}} : {})}
      />
  </ProductionWorkbenchRoot>}</DeveloperModeProvider>;
  renderApplication(<AppRuntimeProvider runtime={runtime}><StructuredResourceStartupGate runtime={runtime}>{runtime.mode === "preview" ? <SpeechInputAdapterProvider adapter={writingDemoSpeechAdapter}>{app}</SpeechInputAdapterProvider> : app}</StructuredResourceStartupGate></AppRuntimeProvider>);
}
void bootstrap().catch(error => { if (rootElement) rootElement.textContent = error instanceof Error ? error.message : "应用启动失败"; });

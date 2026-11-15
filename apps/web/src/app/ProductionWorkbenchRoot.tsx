import {StrictMode, type ReactNode} from "react";
import {NotesProvider} from "../core/features/notes/NotesClient";
import {AgentQueueProvider} from "../core/features/agent/AgentQueueProvider";
import {AgentSteeringProvider} from "../core/features/agent/AgentSteeringProvider";
import {AgentDraftProvider} from "../core/features/agent/AgentDraftProvider";
import {localConversationDraftPersistence} from "../core/features/agent/conversationDraftPersistence";
import {getAppStorage} from "../core/runtime/appStorage";

/** Production and browser previews share these Core-owned providers. */
export function ProductionWorkbenchRoot({children}: {readonly children: ReactNode}) {
  return <StrictMode><NotesProvider><AgentQueueProvider><AgentSteeringProvider>
    <AgentDraftProvider persistence={localConversationDraftPersistence(getAppStorage())}>{children}</AgentDraftProvider>
  </AgentSteeringProvider></AgentQueueProvider></NotesProvider></StrictMode>;
}

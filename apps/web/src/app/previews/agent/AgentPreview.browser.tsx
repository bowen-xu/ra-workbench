/** Application-owned browser test data entry. No normal App route imports it.
 * Only finite synthetic IO and lifecycle methods cross this test boundary. */
export {
  configureAgentPreview, configureLiveAgentResponse, holdLiveAgentOutput, liveAgentTurnIds,
  configureFailedAgentHistory, installLiveContextCatalog, holdAgentMetadata, agentMetadataBlocked, releaseAgentMetadata,
  installAgentStartFailure, createPreviewNote, createBrowserLinkNote,
  configureBrowserOpenFailure, recordBrowserOpenings, readBrowserOpenings, readIndependentSubmissionTexts,
} from "./LiveAgentFixture";

export {
  holdLauncherPreparation, launcherPreparationReached, failHeldLauncherPreparation,
  refreshLauncherActivity, setLauncherInternalActivity, selectLauncherOpenAiModel,
} from "./LauncherPreviewFixture";

export {
  seedToolJournal, preparePendingToolJournal, configureToolResources,
  readToolHistoryCallIds, selectWritingJournal, originalToolHistorySteps,
} from "./ToolJournalFixture";
export type {ToolJournalFixture, ToolJournalStep} from "./ToolJournalFixture";

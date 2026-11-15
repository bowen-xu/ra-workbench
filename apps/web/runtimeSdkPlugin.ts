import type {Plugin} from "vite";
import path from "node:path";
import {fileURLToPath} from "node:url";
import surface from "../../docs/architecture/runtime-note-sdk-surface.json" with {type: "json"};

const repository = fileURLToPath(new URL("../../", import.meta.url));
const prefix = "virtual:workbench-runtime-sdk/";
const subpaths: Readonly<Record<string, string>> = {
  "@research-assistant/core/notes": "core-notes",
  "@research-assistant/data/notes": "data-notes",
  "core/public/agentContext/AgentContextProvider": "agent-context",
  "core/public/agentContext/sourceNavigation": "source-navigation",
  "core/public/configuration": "configuration-hooks",
  "core/public/domainCapabilities": "domain-capabilities",
  "core/public/feedback": "feedback",
  "core/public/layout": "layout",
  "core/public/noteDiagnostics": "note-diagnostics",
  "core/public/noteDocuments": "note-documents",
  "core/public/noteResources": "note-resources",
  "core/public/noteStorage": "note-storage",
  "core/public/noteTransfer": "note-transfer",
  "core/public/overlay": "overlay",
  "core/public/platform": "platform",
  "core/public/reorder": "reorder",
  "core/public/workbench": "workbench",
  "core/public/workspaceState": "workspace-state",
};

interface Facade {
  readonly name: string;
  readonly source: string;
  readonly exports: readonly string[];
}

// The consumer inventory counts import declarations. The same existing
// metadata actions/events also cross re-export declarations in that consumer.
const reexports: Readonly<Record<string, readonly string[]>> = {
  "note-documents": ["NOTE_METADATA_CHANGED", "NOTE_PAGES_DELETED"],
  "note-resources": ["applyNotePageStatus", "publishNoteMetadataReceipt"],
};
// C02's finite literature consumers extend six existing facades. Keep these
// separate from the historical A00 values and A04/A05 note increments.
const literatureRuntimeExtensions: Readonly<Record<string, readonly string[]>> = {
  "core-notes": ["isPositionReadingNote", "readingNoteSourceAt"],
  "feedback": ["useNotificationState"],
  "note-resources": ["useNoteAnnotations", "useLiteratureNotes"],
  "overlay": ["OverlayPortal", "Popover", "Dialog", "useOverlayTarget"],
  "platform": ["open"],
  "reorder": ["ReorderableListRow", "ReorderDragHandle"],
  "workbench": ["usePublishWorkspaceTabs"],
};
// Existing Core producers consumed by the three canonical writing roots.
const writingRuntimeExtensions: Readonly<Record<string, readonly string[]>> = {
  "core-notes": ["blocksToEditor", "decodeNoteBlocks", "editorToBlocks", "isAllowedNoteHref", "noteImageDisplayName"],
  "feedback": ["NotificationEffect"],
  "note-documents": ["NotePageToolbar", "openNoteDocumentTab"],
  "platform": ["downloadDir", "join"],
  "source-navigation": ["openSourceLink"],
  "workspace-state": ["workspaceBoolean"],
  "domain-capabilities": ["getOptionalLiteratureGateway"],
};
const facades: readonly Facade[] = [
  ...surface.modules.filter(value => value.runtime.length).map(value => {
    const name = subpaths[value.module];
    if (!name) throw new Error("runtime_sdk_unmapped_surface: " + value.module);
    return {name, source: value.module.startsWith("core/")
      ? fileURLToPath(new URL("./src/" + value.module + ".ts", import.meta.url)) : value.module,
    exports: [...new Set([...value.runtime, ...(reexports[name] ?? []),
      ...surface.runtimeExtensions.filter(extension => extension.module === value.module).flatMap(extension => extension.runtime),
      ...(literatureRuntimeExtensions[name] ?? []), ...(writingRuntimeExtensions[name] ?? [])])]};
  }),
  // These five C00 consumer groups are new SDK leaves, not changes to A00's
  // frozen note surface. Static bindings below derive from this same table.
  {name: "environment", source: fileURLToPath(new URL("./src/core/public/pluginRuntimeEnvironment.ts", import.meta.url)),
    exports: ["isAppPreview", "isDesktopRuntime", "getRuntimeEnvironment", "ensureRuntimeStructuredResources", "hasPlatformCapabilities",
      "getRuntimeStorage", "getNativePreviewCapabilities", "captureNewPreviewOwnerIntent", "freezeNewPreviewOwnerIntent"]},
  {name: "storage", source: fileURLToPath(new URL("./src/core/public/storage.ts", import.meta.url)),
    exports: ["getAppStorage", "flushStorage", "storagePersistence"]},
  {name: "literature-runtime", source: fileURLToPath(new URL("./src/core/public/literatureRuntime.ts", import.meta.url)),
    exports: ["resolveLiteratureRuntimeGateway", "resolveCollectionRuntimeGateway", "resolveNalRuntimeGateway", "resolveNalQueryRuntimeGateway",
      "resolveKnowledgeRuntimeGateway", "resolveEvidenceRuntimeGateway", "resolveEvidenceMatrixRuntimeGateway", "resolveDerivedViewRuntimeGateway",
      "resolveReferenceRecordRuntimeGateway", "resolveResearchExportRuntimeGateway"]},
  {name: "shortcuts", source: fileURLToPath(new URL("./src/core/public/settings.ts", import.meta.url)),
    exports: ["formatShortcut", "isShortcutMac"]},
  {name: "settings", source: fileURLToPath(new URL("./src/core/public/settings.ts", import.meta.url)),
    exports: ["SettingRow", "SettingGroup", "useDeveloperMode", "SystemDocumentDialog"]},
  {name: "agent-ui", source: fileURLToPath(new URL("./src/core/public/pluginAgentUi.ts", import.meta.url)),
    exports: ["SelectionAskAiButton", "SelectionAskAiEditor", "submitEvidenceAnnotation", "openAgentConversation", "useAgentCanvasMessage", "captureEvidenceImage"]},
  {name: "block-controls", source: fileURLToPath(new URL("./src/core/public/blockControls.ts", import.meta.url)),
    exports: ["BlockControlsProvider", "FloatingBlockControls", "NoteActionMenu", "NoteActionMenuItem", "BlockAddButton", "BlockDragButton", "useBlockControlsRoot", "startBlockPointerDrag", "getEditorScrollContainer", "useBlockDragAutoScroll"]},
  {name: "ui", source: fileURLToPath(new URL("./src/core/public/ui.ts", import.meta.url)),
    exports: ["ContentTrustIndicator", "ContentTrustGroupControls", "CitationStyleSelect", "MouseLeftShortcut", "InlineReference"]},
  {name: "structured-field-colors", source: fileURLToPath(new URL("./src/core/public/structuredFieldColors.ts", import.meta.url)),
    exports: ["READING_FIELD_PALETTE", "readingFieldColor"]},
  {name: "text-documents", source: fileURLToPath(new URL("./src/core/public/textEditor.ts", import.meta.url)),
    exports: ["openTextDocument"]},
  {name: "model-route-picker", source: fileURLToPath(new URL("./src/core/public/modelRoutePicker.ts", import.meta.url)),
    exports: ["LlmRoutePicker"]},
  {name: "pdf", source: fileURLToPath(new URL("./src/core/public/pdf.ts", import.meta.url)),
    exports: ["PdfViewController", "PDF_BACKGROUND_SUSPEND_MS", "PdfPageControls", "usePdfThumbnailCacheWindow", "PdfView", "PdfViewAdapter", "PdfViewControls"]},
  {name: "pdf-geometry", source: fileURLToPath(new URL("./src/core/public/pdfGeometry.ts", import.meta.url)),
    exports: ["positionPdfSelectionPopover", "clampPdfPageIndex", "parsePdfPageInput"]},
  {name: "library-capture", source: fileURLToPath(new URL("./src/core/public/libraryCapture.ts", import.meta.url)),
    exports: ["listenBrowserCaptures", "subscribeWindowFileDrop", "desktopDropClientPosition"]},
  {name: "docling", source: fileURLToPath(new URL("./src/core/public/docling.ts", import.meta.url)),
    exports: ["getDoclingParserGateway", "doclingErrorMessage", "runDoclingParse"]},
  {name: "dictionary", source: fileURLToPath(new URL("./src/core/public/dictionary.ts", import.meta.url)),
    exports: ["getWordExplanationGateway", "createDictionaryGateway", "DictionaryLookup", "WordContextExplanation", "getWordExplanationStore"]},
  {name: "annotation-notes", source: fileURLToPath(new URL("./src/core/public/annotationNotes.ts", import.meta.url)),
    exports: ["listBrowserNotePageIds", "ensureBrowserAnnotationNote"]},
  {name: "documents", source: fileURLToPath(new URL("./src/core/public/documents.ts", import.meta.url)),
    exports: ["getNoteDocumentGateway"]},
  {name: "document-recovery", source: fileURLToPath(new URL("./src/core/public/documentRecovery.ts", import.meta.url)),
    exports: ["clearRecoveryDraft", "createRecoveryDraft", "loadRecoveryDraft", "saveRecoveryDraft", "EDITOR_AUTOSAVE_IDLE_MS"]},
  {name: "hbs", source: fileURLToPath(new URL("./src/core/public/hbs.ts", import.meta.url)),
    exports: ["HbsStructuredEditor", "directTextDiff", "getHbsDocumentGateway", "getPendingHbsDocumentCommit"]},
  {name: "literature-notes", source: fileURLToPath(new URL("./src/core/public/literatureNotes.ts", import.meta.url)),
    exports: ["stageLegacyAnnotationDocument", "requestLiteratureNoteTrash"]},
  {name: "structured-resources", source: fileURLToPath(new URL("./src/core/public/structuredResources.ts", import.meta.url)),
    exports: ["getStructuredResourceGateway", "structuredWriteValidationDiagnostic"]},
  {name: "ai-assistance", source: fileURLToPath(new URL("./src/core/public/pluginAiAssistance.ts", import.meta.url)),
    exports: ["getAiAssistanceGateway", "aiAssistanceErrorMessage"]},
  {name: "concept-retrieval", source: fileURLToPath(new URL("./src/core/public/conceptRetrieval.ts", import.meta.url)),
    exports: ["PaperConceptRetrievalControl"]},
  // Canonical writing consumers retain the same Core UI and runtime owners.
  {name: "block-content", source: fileURLToPath(new URL("./src/core/public/blockContent.ts", import.meta.url)),
    exports: ["ImageBlockPresentation", "ManagedAssetIngress", "NoteEditorSurface", "baseBlockEditorSchema", "blockEditorDefaults", "withCaptionBlocks", "withoutCaptionBlocks"]},
  {name: "document-windows", source: fileURLToPath(new URL("./src/core/public/documentWindows.ts", import.meta.url)),
    exports: ["NoteFloatingWindow"]},
  {name: "editor-review", source: fileURLToPath(new URL("./src/core/public/editorReview.ts", import.meta.url)),
    exports: ["EditorDiffControls", "EditorDiffHighlight", "chunkActionDecorations", "resolveTextChunk", "resolveTextDiff", "textChanges", "textDiffResolution", "textReviewBaseline"]},
  {name: "code-editor-extensions", source: fileURLToPath(new URL("./src/core/public/codeEditorExtensions.ts", import.meta.url)),
    exports: ["createEditorSearchPanel", "editorSearchScope", "latexLanguage", "virtualCompletion"]},
  {name: "writing-storage", source: fileURLToPath(new URL("./src/core/public/writingStorage.ts", import.meta.url)),
    exports: ["decodeWritingBody", "decodeWritingDraft", "decodeWritingFile", "decodeWritingProject", "decodeWritingSession", "writingFileKey", "writingRecords"]},
  {name: "writing-runtime", source: fileURLToPath(new URL("./src/core/public/writingRuntime.ts", import.meta.url)),
    exports: ["getWritingNoteAssetNativeGateway", "resolveWritingCheckpointRuntimeGateway", "resolveWritingRuntimeGateway"]},
  {name: "git-versions", source: fileURLToPath(new URL("./src/core/public/gitVersions.ts", import.meta.url)),
    exports: ["getManuscriptVersionGateway"]},
  {name: "git-worktrees", source: fileURLToPath(new URL("./src/core/public/gitWorktrees.ts", import.meta.url)),
    exports: ["getGitWorktreeGateway"]},
  {name: "text-diff-stats", source: fileURLToPath(new URL("./src/core/public/textDiffStats.ts", import.meta.url)),
    exports: ["countWritingChangedLines"]},
  {name: "agent-gateways", source: fileURLToPath(new URL("./src/core/public/agent.ts", import.meta.url)),
    exports: ["agentApplicationGateway", "agentCatalogGateway", "agentCompactionGateway", "agentCompileResultGateway", "agentCreationGateway", "agentEditorRestorationGateway", "agentInputQueueGateway", "agentInputQueuePreviewGateway", "agentInstructionTrustGateway", "agentMetadataGateway", "agentPendingWritingApplicationGateway", "agentPendingWritingUndoGateway", "agentProposalGateway", "agentSessionGateway", "agentSteeringGateway", "agentSubmissionGateway", "agentSubtaskGateway", "agentTurnGateway", "agentUndoGateway", "agentWorkspaceDiscardGateway", "agentWorkspaceGateway", "agentWorkspaceReviewGateway", "agentWorkspaceSuccessorGateway", "agentWritingFileDeliveryGateway", "getAgentUndoEditorRestorationGateway"]},
  {name: "agent-delivery", source: fileURLToPath(new URL("./src/core/public/pluginAgentDelivery.ts", import.meta.url)),
    exports: ["writingDeliveryToolGateway", "writingSuccessorToolGateway"]},
  {name: "agent-diagnostics", source: fileURLToPath(new URL("./src/core/public/pluginAgentDiagnostics.ts", import.meta.url)),
    exports: ["agentFailureDiagnostics", "agentHistoryErrorMessage", "diagnosticError", "writingPreparationSourceMessage"]},
  {name: "agent-drafts", source: fileURLToPath(new URL("./src/core/public/pluginAgentDrafts.ts", import.meta.url)),
    exports: ["useAgentDraftStore", "useConversationDraft", "decodeConversationDraftAcknowledgmentToken", "queueDraftEdit"]},
  {name: "agent-writing", source: fileURLToPath(new URL("./src/core/public/agentWriting.ts", import.meta.url)),
    exports: ["ReviewedSubmissionController", "ReviewedSubmissionOwnerRegistry", "acquireWritingSubmissionController", "captureWritingDraftReceipt", "decodeWritingDraftAcknowledgment", "hasPendingWritingSubmissions", "releaseWritingSubmissionController", "sameWritingQueueCancellationIntent", "unownedWritingDraftReceipt", "writingDraftAcknowledgments", "writingPreparationDomainFields", "writingPreparationMessage", "writingPreparationRejection", "writingQueueCancellationMessage", "writingQueueCancellationRecovery", "writingQueuePreparationMessage", "writingQueuedStartMessage", "writingQueuedStartQueue", "writingWorkspacePreparationMessage"]},
  {name: "agent-conversations", source: fileURLToPath(new URL("./src/core/public/pluginAgentConversations.ts", import.meta.url)),
    exports: ["BoundConversationList", "ConversationCreationRecovery", "conversationCreationMessage", "ConversationForkOrigin", "clearAgentSessionSelection", "readAgentSessionSelection", "writeAgentSessionSelection", "nameInitialConversation", "pendingBoundConversationId", "readBoundCreation", "reserveBoundConversation", "AgentConversationController", "projectConversationMessages", "useAgentConversation", "useBoundConversationCreation", "useConversationForkActions", "useConversationMetadata"]},
  {name: "agent-context-controls", source: fileURLToPath(new URL("./src/core/public/pluginAgentContextControls.ts", import.meta.url)),
    exports: ["agentContextGateway", "useAgentContextUsage", "useContextContribution", "useConversationSteeringHandoff", "useHiddenHandoffQueueInputs"]},
  {name: "agent-submission", source: fileURLToPath(new URL("./src/core/public/pluginAgentSubmission.ts", import.meta.url)),
    exports: ["AgentPendingSteeringBubbles", "AgentPendingSubmissionBubble", "hasConfirmedSubmissionMessage", "useAgentQueueOwners", "AgentSteeringHandoffRecovery", "useAgentSteeringOwners", "AgentSteeringReceipt", "AgentSubmissionFeedback", "readComposerCommand", "AGENT_START_FAILURE", "recoverAgentPreparationFailure", "dispatchQueuedAgentCommand", "holdRejectedQueuePreparation", "writingQueuePreparingPresentation", "pauseRejectedSteerQueue", "recoverRejectedSteer", "reconcileSteeringTransfer", "useAgentInputQueue", "useAgentQueueAutoPrepare", "useAgentSteering"]},
  {name: "agent-presentation", source: fileURLToPath(new URL("./src/core/public/pluginAgentPresentation.ts", import.meta.url)),
    exports: ["AgentChangeContribution", "WritingAgentPanel", "AgentTransientFailure", "AgentRecoveryMessage", "readMessageInspection", "useAgentChildTasks", "useAgentHostLifecycle", "useAgentMountedGuard", "WritingAgentTaskStack"]},
  {name: "agent-canvas", source: fileURLToPath(new URL("./src/core/public/pluginAgentCanvas.ts", import.meta.url)),
    exports: ["AgentCanvasHost", "useAgentCanvasExternalSubmission"]},
  // Actual C00 bare imports share Core's public package instances. They are
  // output entries for independent builders, never synthetic SDK subpaths.
  {name: "core-hbs", source: "@research-assistant/core/hbs",
    exports: ["createStructuredDocumentAdapter", "DocumentVersionConflict", "HbsSession"]},
  {name: "data-hbs", source: "@research-assistant/data/hbs",
    exports: ["createDocumentPort", "createDocumentPreservationPort"]},
  {name: "core-agent", source: "@research-assistant/core/agent", exports: ["PromptTemplate", "decodeAgentEvidenceInput", "AgentWorkspaceDecodeError", "decodeAgentUndoRequest", "executionModeForModel", "sameAgentWritingStartRequest", "sameAgentWritingQueuePreviewRequest", "sameAgentWritingQueuedStartRequest", "sameAgentWritingPrepareRequest", "agentTurnUnconfirmedChildCreations", "decodeAgentWritingQueuedCompactionPrepareRequest", "decodeAgentWritingQueuedCompactionPreview", "agentWritingCompactionStartRequest", "decodeAgentWritingQueuedCompactionReadRequest", "sameAgentWritingCompactionPrepareRequest", "decodeAgentWritingQueuedCompactionSettleRequest", "decodeAgentWritingCompactionPrepareRequest", "decodeAgentWritingCompactionPreview", "decodeAgentWritingPrepareResult", "decodeAgentCompactionStartRequest", "decodeAgentStartResponse", "decodeAgentWritingCompactionPrepareStatusResponse", "decodeAgentWritingCompactionStartStatusResponse", "sameAgentCompactionStartRequest", "decodeAgentSqlImmediateFailure", "agentConversationRequiresToolRecovery", "agentDraftInput", "decodeAgentCompileResult"]},
  {name: "core-ai", source: "@research-assistant/core/ai", exports: ["DEFAULT_PORTRAIT_CATALOG", "catalogDimensions", "decodePaperTemplateSnapshot", "decodePortraitCatalog", "overviewFieldTitle", "portraitDimensions", "portraitGenerationGoal", "portraitTagDefinition", "selectPaperPortraitTemplate", "togglePortraitTag"]},
  {name: "core-citation", source: "@research-assistant/core/citation", exports: ["exportLiterature", "formatBibliography", "formatInTextCitation", "parseBibtex"]},
  {name: "core-collections", source: "@research-assistant/core/collections", exports: ["applyCollectionMutation", "decodeCollectionState", "emptyCollectionState"]},
  {name: "core-content-trust", source: "@research-assistant/core/content-trust", exports: ["aggregateContentTrust"]},
  {name: "core-dictionary", source: "@research-assistant/core/dictionary", exports: ["isDictionaryQuery", "normalizeDictionaryQuery", "wordExplanationForSelection"]},
  {name: "core-knowledge", source: "@research-assistant/core/knowledge", exports: ["RELATION_NODE_KINDS", "RELATION_PREDICATE_KINDS", "STRUCTURED_READING_UNIT_KINDS", "assertAvailableConceptName", "importedConceptName", "splitConceptName"]},
  {name: "core-literature", source: "@research-assistant/core/literature", exports: ["LITERATURE_TYPE_OPTIONS", "applyLiteratureCreatorInput", "createLiteratureCreatorInput", "emptyLiteratureDraft", "formatLiteratureCreators", "primaryCreatorRoleForLiteratureType", "updateLiteratureCreatorInput"]},
  {name: "core-nal", source: "@research-assistant/core/nal", exports: ["NalQueryCollections", "applyNalMutation", "decodeNalAnnotation", "decodeNalQueryCache", "decodeNalQueryDefinitions", "decodeNalState", "emptyNalState", "formatNalStatement", "formatNalTerm", "isNalQueryGroup", "nalAtoms", "nalConceptKind", "nalQueryLeaves", "nalQueryVariables", "parseNalConditions", "validateNalQueryBlocks", "validateNalTerm"]},
  {name: "core-reader", source: "@research-assistant/core/reader", exports: ["DEFAULT_READER_UI_SESSION_STATE", "ReaderNavigationHistory", "decodeReaderUiSessionState", "decodeReaderViewStateV2", "isReaderCommandEnabled", "nextReaderLifecycleState", "normalizeReaderLocation", "readerCancellationCommand", "readerCommandForKey"]},
  {name: "core-source", source: "@research-assistant/core/source", exports: ["formatSourceLink"]},
  {name: "core-structured-resources", source: "@research-assistant/core/structured-resources", exports: ["structuredResourceDiffFields"]},
  {name: "core-synthesis", source: "@research-assistant/core/synthesis", exports: ["REFERENCE_FIELD_NAMES", "detectDoclingReferences"]},
  {name: "core-document-review", source: "@research-assistant/core/document-review", exports: ["sameReviewDocument", "persistReviewSnapshots"]},
  {name: "codemirror-state", source: "@codemirror/state", exports: ["ChangeSet", "Compartment", "EditorSelection", "EditorState", "Facet", "Prec", "RangeSet", "RangeSetBuilder", "StateEffect", "StateField", "Text", "Transaction", "combineConfig"]},
  {name: "codemirror-view", source: "@codemirror/view", exports: ["Decoration", "EditorView", "GutterMarker", "ViewPlugin", "WidgetType", "activateHover", "drawSelection", "dropCursor", "getPanel", "gutter", "highlightActiveLine", "highlightActiveLineGutter", "hoverTooltip", "keymap", "lineNumbers", "logException", "rectangularSelection", "showPanel", "showTooltip"]},
  {name: "codemirror-merge", source: "@codemirror/merge", exports: ["diff", "getChunks", "getOriginalDoc", "originalDocChangeEffect", "unifiedMergeView"]},
  {name: "codemirror-commands", source: "@codemirror/commands", exports: ["copyLineDown", "defaultKeymap", "deleteLine", "history", "historyKeymap", "indentLess", "indentMore", "invertedEffects", "isolateHistory", "redo", "undo"]},
  {name: "codemirror-search", source: "@codemirror/search", exports: ["gotoLine", "highlightSelectionMatches", "openSearchPanel", "search", "searchKeymap"]},
  {name: "codemirror-language", source: "@codemirror/language", exports: ["StreamLanguage", "bracketMatching", "defaultHighlightStyle", "foldAll", "foldGutter", "foldKeymap", "indentOnInput", "indentUnit", "syntaxHighlighting", "toggleFold", "unfoldAll"]},
  {name: "codemirror-autocomplete", source: "@codemirror/autocomplete", exports: ["acceptCompletion", "clearSnippet", "closeBrackets", "closeBracketsKeymap", "closeCompletion", "completionKeymap", "insertCompletionText", "nextSnippetField", "pickedCompletion", "prevSnippetField", "snippet", "snippetKeymap", "startCompletion"]},
  {name: "blocknote-react", source: "@blocknote/react", exports: ["createReactBlockSpec", "useCreateBlockNote"]},
  // Exact third-party imports already consumed by the C00 literature roots.
  // The worker's default value is Core's emitted asset URL, not a second worker.
  {name: "ariakit-react", source: "@ariakit/react", exports: ["Tab", "TabList", "TabPanel", "TabProvider", "PopoverDisclosure", "PopoverProvider", "usePopoverStore", "useStoreState", "DialogHeading"]},
  {name: "pdfjs", source: "pdfjs-dist/legacy/build/pdf.mjs", exports: ["GlobalWorkerOptions", "getDocument"]},
  {name: "pdfjs-worker", source: "pdfjs-dist/legacy/build/pdf.worker.mjs?url", exports: ["default"]},
  {name: "react", source: "react", exports: ["default", "Activity", "Children", "Component", "Fragment", "Profiler", "PureComponent", "StrictMode", "Suspense",
    "act", "cache", "cacheSignal", "cloneElement", "createContext", "createElement", "createRef", "forwardRef", "isValidElement", "lazy", "memo", "startTransition",
    "unstable_useCacheRefresh", "use", "useActionState", "useCallback", "useContext", "useDebugValue", "useDeferredValue", "useEffect", "useEffectEvent", "useId",
    "useImperativeHandle", "useInsertionEffect", "useLayoutEffect", "useMemo", "useOptimistic", "useReducer", "useRef", "useState", "useSyncExternalStore", "useTransition", "version"]},
  {name: "react-jsx-runtime", source: "react/jsx-runtime", exports: ["Fragment", "jsx", "jsxs"]},
  {name: "react-jsx-dev-runtime", source: "react/jsx-dev-runtime", exports: ["Fragment", "jsxDEV"]},
  {name: "react-dom", source: "react-dom", exports: ["default", "createPortal", "flushSync", "preconnect", "prefetchDNS", "preinit", "preinitModule", "preload", "preloadModule",
    "requestFormReset", "unstable_batchedUpdates", "useFormState", "useFormStatus", "version"]},
  {name: "react-dom-client", source: "react-dom/client", exports: ["createRoot", "hydrateRoot", "version"]},
];

const facadeModule = (facade: Facade): string =>
  `export {${facade.exports.join(",")}} from ${JSON.stringify(facade.source)};`;

// The actual C00 classes retain these existing public-package modules.
// Observe their emitted identity with the original owner/chunk guard; this
// creates no session, authority, storage or runtime registry.
const sharedIdentityModules = new Set([
  "packages/core/src/hbs/session.ts", "packages/core/src/hbs/document.ts", "packages/core/src/hbs/adapters.ts",
  "packages/data/src/agent/conversationController.ts", "packages/data/src/agent/reviewedSubmissionController.ts", "packages/data/src/agent/reviewedSubmissionOwner.ts",
  "packages/data/src/hbs/documentPort.ts", "packages/data/src/hbs/documentPreservationPort.ts",
  "packages/core/src/agent/promptTemplate.ts", "packages/core/src/nal/queryCollections.ts", "packages/core/src/reader/history.ts",
].map(source => path.join(repository, source)));

/** The static comparison product consumes the same SDK values from the same
 * Core modules. Derive this binding from the runtime facade table, rather than
 * maintaining another export map or evaluating declaration-only SDK modules. */
export function workbenchSdkSourcePlugin(): Plugin {
  const sourcePrefix = "virtual:workbench-source-sdk/";
  const bindings = facades.filter(facade => facade.source.startsWith(path.join(repository, "apps/web/src/core/public") + path.sep));
  return {
    name: "workbench-sdk-source-bindings",
    enforce: "pre",
    api: {facades: bindings.map(facade => ({...facade, source: facade.source.replace(repository, "")}))},
    resolveId(id) {
      const facade = bindings.find(value => id === "@research-assistant/workbench-sdk/" + value.name);
      if (facade) return "\0" + sourcePrefix + facade.name;
    },
    load(id) {
      const facade = bindings.find(value => id === "\0" + sourcePrefix + value.name);
      if (facade) return facadeModule(facade);
    },
  };
}

/** Facades are entries in the Core build graph. Re-exporting the actual modules
 * here preserves their React/context/store identity across Core and plugins. */
export function runtimeSdkPlugin(enabled = process.env.RESEARCH_ASSISTANT_RUNTIME_PLUGINS,
  selection = process.env.RESEARCH_ASSISTANT_BUILTIN_PLUGINS): Plugin | undefined {
  if (enabled === undefined || enabled === "0") return;
  if (enabled !== "1") throw new Error("runtime_sdk_invalid_mode: expected 1 or 0");
  if (selection !== "none") throw new Error("runtime_sdk_requires_core_only: set RESEARCH_ASSISTANT_BUILTIN_PLUGINS=none");
  return {
    name: "workbench-runtime-sdk",
    api: {facades: facades.map(value => ({...value, source: value.source.replace(repository, "")}))},
    config(_config, environment) {
      if (environment.command !== "build") throw new Error("runtime_sdk_requires_build: serve the constructed Core product");
      // Native ESM imports of SDK entries do not run Vite's dynamic CSS
      // preloader. Core HTML must load their original shared UI styles too.
      return {build: {cssCodeSplit: false, rolldownOptions: {
        input: {index: fileURLToPath(new URL("./index.html", import.meta.url)),
          ...Object.fromEntries(facades.map(value => ["sdk/v1/" + value.name, prefix + value.name]))},
        preserveEntrySignatures: "strict",
        output: {entryFileNames: chunk => chunk.name.startsWith("sdk/v1/") ? "[name].js" : "assets/[name]-[hash].js"},
      }}};
    },
    configResolved(config) {
      const output = path.resolve(config.root, config.build.outDir);
      if (output !== path.join(repository, "apps/web/dist")
        && !output.startsWith(path.join(repository, ".tmp") + path.sep))
        throw new Error("runtime_sdk_output_invalid: use the normal Web dist or a repository .tmp output");
    },
    resolveId(id) {if (id.startsWith(prefix) && facades.some(value => id === prefix + value.name)) return "\0" + id;},
    load(id) {
      const facade = facades.find(value => id === "\0" + prefix + value.name);
      if (facade) return facadeModule(facade);
    },
    generateBundle(_options, bundle) {
      const chunks = Object.values(bundle).filter(value => value.type === "chunk");
      const modules = [...this.getModuleIds()].filter(id => !id.startsWith("\0")).sort();
      const forbidden = modules.filter(id => /\/(?:src\/plugins\/[^/]+\/|plugins\/[^/]+\/src\/)/u.test(id));
      if (forbidden.length) this.error("runtime_sdk_plugin_implementation_in_core:\n" + forbidden.join("\n"));
      const emitted = facades.map(facade => {
        const fileName = "sdk/v1/" + facade.name + ".js";
        const chunk = chunks.find(value => value.fileName === fileName);
        if (!chunk || facade.exports.some(name => !chunk.exports.includes(name))) this.error("runtime_sdk_missing_exports: " + fileName);
        return {name: facade.name, fileName, exports: chunk.exports, imports: chunk.imports};
      });
      const owners = modules.filter(id => sharedIdentityModules.has(id) || /\/src\/core\/(?:features\/notes\/NotesClient|runtime\/app(?:Runtime|Storage)|public\/literatureRuntime)\.tsx?$|\/node_modules\/react\/(?:index|cjs\/react\.production)\.js$/u.test(id))
        .map(id => ({module: id.replace(repository, ""), chunks: chunks.filter(chunk => id in chunk.modules).map(chunk => chunk.fileName)}));
      if (owners.some(owner => owner.chunks.length !== 1)) this.error("runtime_sdk_owner_identity_violation");
      this.emitFile({type: "asset", fileName: "runtime-sdk.json", source: JSON.stringify({
        version: 1, facadeBuild: "same-core-multi-entry", facades: emitted, owners,
        modules: modules.map(id => id.replace(repository, "")),
      }, null, 2)});
    },
  };
}

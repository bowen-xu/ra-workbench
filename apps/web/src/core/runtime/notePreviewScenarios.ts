export type NotePreviewScenario = "open" | "conflict" | "review-conflict" | "stale-proposal" | "ai-write" | "save-failure" | "resolution-failure" | "resolution-unknown" | "reopen";
export interface NotePreviewScenarios { run(scenario: NotePreviewScenario): Promise<void> }

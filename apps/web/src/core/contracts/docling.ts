export interface DoclingParseTaskStatus {
  readonly id: string;
  readonly state: "running" | "cancelling" | "succeeded" | "failed" | "cancelled";
  readonly stage: string;
  readonly completedPages: number;
  readonly totalPages: number;
  readonly parseRunId?: string | null;
  readonly error?: { readonly code: string; readonly message: string } | null;
}

export interface DoclingSettings {
  readonly enabled: boolean;
  readonly modelManifestUrl: string;
  readonly defaultPort: number;
  readonly fallbackStartPort: number;
  readonly fallbackEndPort: number;
  readonly idleShutdownMs: number;
  readonly requestTimeoutMs: number;
  readonly updatedAt: string;
}

export type DoclingSettingsDraft = Omit<DoclingSettings, "updatedAt">;

export interface DoclingServiceStatus {
  readonly state: string;
  readonly baseUrl?: string | null;
  readonly port?: number;
  readonly pid?: number;
  readonly instanceId?: string;
  readonly lastError?: string;
  readonly registryPath?: string;
  readonly activeClientCount?: number | null;
  readonly activeRequestCount?: number | null;
}

export interface DoclingPingResult {
  readonly status: DoclingServiceStatus;
  readonly latencyMs: number;
}

export interface DoclingParseRunSummary {
  readonly id: string;
  readonly itemId: string;
  readonly attachmentId: string;
  readonly contentSha256: string;
  readonly serviceBaseUrl: string;
  readonly doclingStatus: string;
  readonly parseStatus: "running" | "success" | "partial_success" | "failure";
  readonly errorMessage?: string;
  readonly processingTimeSeconds?: number;
  readonly createdAt: string;
  readonly finishedAt: string;
}

export interface DoclingParseRunDetail extends DoclingParseRunSummary {
  readonly jsonContent?: unknown;
  readonly rawResponse: unknown;
}

export interface DoclingOverlayRect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export interface DoclingOverlayRegion {
  readonly pageIndex: number;
  readonly rect: DoclingOverlayRect;
}

export interface DoclingOverlayItem {
  readonly id: string;
  readonly kind: string;
  readonly label?: string;
  readonly textPreview?: string;
  readonly regions: readonly DoclingOverlayRegion[];
  readonly rawRef: string;
}

export interface DoclingPageOverlay {
  readonly attachmentId: string;
  readonly parseRunId: string;
  readonly pageIndex: number;
  readonly pageSize?: {
    readonly width: number;
    readonly height: number;
  };
  readonly items: readonly DoclingOverlayItem[];
}

export interface DoclingGateway {
  startParseTask(itemId: string, attachmentId: string, options?: { readonly force: boolean }): Promise<DoclingParseTaskStatus>;
  getParseTask(taskId: string): Promise<DoclingParseTaskStatus>;
  cancelParseTask(taskId: string): Promise<DoclingParseTaskStatus>;
  getModelStorage(): Promise<DoclingModelStorage>;
  deleteModels(): Promise<DoclingModelStorage>;
  getModelStatus(): Promise<DoclingModelStatus>;
  checkModelUpdate(): Promise<DoclingModelStatus>;
  downloadModels(): Promise<DoclingModelStatus>;
  getSettings(): Promise<DoclingSettings>;
  saveSettings(draft: DoclingSettingsDraft): Promise<DoclingSettings>;
  getStatus(): Promise<DoclingServiceStatus>;
  pingService(): Promise<DoclingPingResult>;
  connectService(): Promise<DoclingServiceStatus>;
  disconnectService(): Promise<DoclingServiceStatus>;
  parseAttachment(itemId: string, attachmentId: string, options?: { readonly force: boolean }): Promise<DoclingParseRunDetail>;
  listParseRuns(itemId: string): Promise<readonly DoclingParseRunSummary[]>;
  getParseRun(runId: string): Promise<DoclingParseRunDetail>;
  getPageOverlay(attachmentId: string, pageIndex: number, parseRunId?: string): Promise<DoclingPageOverlay>;
}

export interface DoclingModelStatus {
  readonly state: string;
  readonly installedVersion?: string | null;
  readonly availableVersion?: string | null;
  readonly installedIdentity?: string | null;
  readonly availableIdentity?: string | null;
  readonly downloadedBytes: number;
  readonly totalBytes: number;
  readonly lastError?: string | null;
}

export interface DoclingModelStorage {
  readonly totalBytes: number;
  readonly installedVersion?: string | null;
  readonly downloading: boolean;
}

/** Parsing is independently consumable; model installation and service settings remain Core-owned. */
export type DoclingParserGateway = Pick<DoclingGateway,
  "startParseTask" | "getParseTask" | "cancelParseTask" | "parseAttachment" |
  "listParseRuns" | "getParseRun" | "getPageOverlay"
>;
/** The parser owns execution; its caller supplies only display metadata. */
export interface DoclingAttachmentMetadataPort {
  describe(itemId: string, attachmentId: string): Promise<{
    readonly paperTitle: string;
    readonly fileName: string;
  }>;
}

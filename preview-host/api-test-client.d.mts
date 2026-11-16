export interface WorkbenchApiTestHost {
  readonly processId: number;
  readonly version: string;
  invoke<T>(command: string, arguments_?: Readonly<Record<string, unknown>>): Promise<T>;
  close(): Promise<void>;
}

/** The application uses an explicit development data directory and runtime package directory. */
export declare function launchWorkbenchApiTestHost(options: {
  readonly executable: string;
  readonly dataDirectory: string;
  readonly pluginDirectory?: string;
  readonly expectedVersion: string;
  readonly environment?: Readonly<Record<string, string | undefined>>;
  readonly timeoutMs?: number;
}): Promise<WorkbenchApiTestHost>;

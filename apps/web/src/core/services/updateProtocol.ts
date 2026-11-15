export interface UpdateStatus {
  readonly revision: number;
  readonly phase: "idle" | "checking" | "upToDate" | "available" | "downloading" | "paused" | "downloaded" | "installing" | "browserOnly" | "failed";
  readonly message: string;
  readonly version: string | null;
  readonly downloaded: number;
  readonly total: number | null;
  readonly readyToInstall: boolean;
}
export const INITIAL_UPDATE_STATUS: UpdateStatus = {
  revision: 0, phase: "idle", message: "检查更新", version: null,
  downloaded: 0, total: null, readyToInstall: false,
};
export interface UpdateGateway {
  status(): Promise<UpdateStatus>;
  subscribe(listener: (status: UpdateStatus) => void): Promise<() => void>;
  check(): Promise<UpdateStatus>;
  download(): Promise<UpdateStatus>;
  pause(paused: boolean): Promise<UpdateStatus>;
  install(): Promise<UpdateStatus>;
}

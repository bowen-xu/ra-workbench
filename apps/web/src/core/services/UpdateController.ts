import { INITIAL_UPDATE_STATUS, type UpdateGateway, type UpdateStatus } from "./updateProtocol";

/** Owns one update session; progress subscribers do not rerender the app shell. */
export class UpdateController {
  #status = INITIAL_UPDATE_STATUS;
  readonly #listeners = new Set<() => void>();
  #installing = false;
  #action = false;
  constructor(readonly gateway: UpdateGateway, readonly prepareInstall: () => Promise<boolean>,
    readonly report: (message: string, error: boolean) => void) {}

  snapshot = (): UpdateStatus => this.#status;
  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };
  async connect(): Promise<() => void> {
    // Subscribe first so a status read cannot hide a newer progress event.
    const unsubscribe = await this.gateway.subscribe(this.accept);
    try { this.accept(await this.gateway.status()); }
    catch (error) { this.fail(error); }
    return unsubscribe;
  }
  accept = (status: UpdateStatus): void => {
    if (status.revision <= this.#status.revision) return;
    const previous = this.#status;
    this.#status = status;
    this.emit();
    if (status.phase === "failed" && previous.phase !== "failed") this.report(status.message, true);
    if ((status.phase === "upToDate" || status.phase === "browserOnly") && previous.phase === "checking") this.report(status.message, false);
    if (status.phase === "downloaded") void this.install();
  };
  private emit(): void { this.#listeners.forEach(listener => listener()); }
  private fail(error: unknown): void {
    const detail = error instanceof Error ? error.message : String(error);
    this.#status = { ...this.#status, phase: "failed", message: `更新未完成：${detail}（错误码：update_client_failed）` };
    this.emit(); this.report(this.#status.message, true);
  }
  private async install(): Promise<void> {
    if (this.#installing) return;
    this.#installing = true;
    try {
      if (!await this.prepareInstall()) throw new Error("编辑内容尚未安全保存，请处理后重试安装");
      this.accept(await this.gateway.install());
    } catch (error) { this.fail(error); }
    finally { this.#installing = false; }
  }
  activate = async (): Promise<void> => {
    if (this.#action) return;
    const status = this.#status;
    if (status.phase === "checking" || status.phase === "installing" || status.phase === "downloaded") return;
    // Download runs independently so pause remains available during its promise.
    if (status.phase === "available" || (status.phase === "failed" && status.version && !status.readyToInstall)) {
      this.#status = { ...status, phase: "downloading", downloaded: 0, total: null, message: "正在开始下载…" };
      this.emit();
      void this.gateway.download().then(this.accept).catch(error => this.fail(error));
      return;
    }
    this.#action = true;
    try {
      if (status.phase === "failed" && status.readyToInstall) await this.install();
      else if (status.phase === "downloading" || status.phase === "paused") this.accept(await this.gateway.pause(status.phase === "downloading"));
      else {
        this.#status = { ...status, phase: "checking", message: "正在检查更新…" }; this.emit();
        this.accept(await this.gateway.check());
      }
    } catch (error) { this.fail(error); }
    finally { this.#action = false; }
  };
}

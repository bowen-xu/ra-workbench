export interface PdfThumbnailJob {
  readonly pageNumber: number;
  readonly canvas: HTMLCanvasElement;
  readonly signal?: AbortSignal;
}

interface QueuedThumbnailJob extends PdfThumbnailJob {
  readonly resolve: () => void;
  readonly reject: (reason: unknown) => void;
}

export class PdfThumbnailQueue {
  readonly #render: (job: PdfThumbnailJob) => Promise<void>;
  readonly #pending: QueuedThumbnailJob[] = [];
  #running = false;
  #paused = false;
  #disposed = false;
  #activeAbortController: AbortController | undefined;
  readonly #idle = new Set<() => void>();

  constructor(render: (job: PdfThumbnailJob) => Promise<void>) {
    this.#render = render;
  }

  enqueue(job: PdfThumbnailJob): Promise<void> {
    if (this.#disposed || job.signal?.aborted === true) return Promise.reject(new DOMException("Thumbnail cancelled", "AbortError"));
    return new Promise<void>((resolve, reject) => {
      this.#pending.push({ ...job, resolve, reject });
      void this.#drain();
    });
  }

  pause(): void {
    this.#paused = true;
    this.#activeAbortController?.abort();
    this.#settlePending();
  }

  resume(): void {
    if (this.#disposed) return;
    this.#paused = false;
    void this.#drain();
  }

  dispose(): void {
    this.#disposed = true;
    this.#paused = true;
    this.#activeAbortController?.abort();
    this.#settlePending();
  }

  /** Cancellation must settle before document-wide PDF.js cleanup. */
  whenIdle(): Promise<void> {
    return this.#running ? new Promise(resolve => this.#idle.add(resolve)) : Promise.resolve();
  }

  async #drain(): Promise<void> {
    if (this.#running || this.#paused || this.#disposed) return;
    this.#running = true;
    try {
      while (!this.#paused && !this.#disposed) {
        const job = this.#pending.shift();
        if (job === undefined) break;
        if (job.signal?.aborted === true) {
          job.reject(new DOMException("Thumbnail cancelled", "AbortError"));
          continue;
        }
        const controller = new AbortController();
        this.#activeAbortController = controller;
        const abort = (): void => controller.abort();
        job.signal?.addEventListener("abort", abort, { once: true });
        try {
          await this.#render({
            pageNumber: job.pageNumber,
            canvas: job.canvas,
            signal: controller.signal,
          });
          if (controller.signal.aborted) job.reject(new DOMException("Thumbnail cancelled", "AbortError"));
          else job.resolve();
        } catch (reason: unknown) {
          if (controller.signal.aborted) job.reject(new DOMException("Thumbnail cancelled", "AbortError"));
          else job.reject(reason);
        } finally {
          job.signal?.removeEventListener("abort", abort);
          if (this.#activeAbortController === controller) this.#activeAbortController = undefined;
        }
      }
    } finally {
      this.#running = false;
      this.#idle.forEach(resolve=>resolve());this.#idle.clear();
      if (!this.#paused && !this.#disposed && this.#pending.length > 0) void this.#drain();
    }
  }

  #settlePending(): void {
    this.#pending.splice(0).forEach((job) => job.reject(new DOMException("Thumbnail cancelled", "AbortError")));
  }
}

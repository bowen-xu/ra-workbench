export interface ReaderCloseBoundary {
  readonly isActive: () => boolean;
  readonly prepare: () => Promise<boolean>;
  readonly prepareClose?: () => Promise<boolean>;
  readonly commit: () => void;
}

export class ReaderCloseCoordinator {
  readonly #boundaries = new Map<string, ReaderCloseBoundary>();
  readonly #closing = new Map<string, Promise<boolean>>();

  register(id: string, boundary: ReaderCloseBoundary): () => void {
    this.#boundaries.set(id, boundary);
    return () => {
      if (this.#boundaries.get(id) === boundary) this.#boundaries.delete(id);
    };
  }

  async close(id: string): Promise<boolean> {
    const existing = this.#closing.get(id);
    if (existing !== undefined) return existing;
    const boundary = this.#boundaries.get(id);
    if (boundary === undefined) return false;
    const operation = (async () => {
      if (!(await (boundary.prepareClose ?? boundary.prepare)())) return false;
      boundary.commit();
      return true;
    })().finally(() => this.#closing.delete(id));
    this.#closing.set(id, operation);
    return operation;
  }

  async closeActive(): Promise<boolean> {
    const active = [...this.#boundaries].find(([, boundary]) => boundary.isActive());
    return active === undefined ? false : this.close(active[0]);
  }

  hasActive(): boolean {
    return [...this.#boundaries.values()].some((boundary) => boundary.isActive());
  }

  async prepareAll(): Promise<boolean> {
    // Preparation can publish state and replace a host's registration. Capture
    // the owners once so an awaited operation cannot extend its own iteration.
    const boundaries = [...this.#boundaries.values()];
    for (const boundary of boundaries) {
      if (!(await boundary.prepare())) return false;
    }
    return true;
  }
}

let coordinator: ReaderCloseCoordinator | undefined;

export function getReaderCloseCoordinator(): ReaderCloseCoordinator {
  coordinator ??= new ReaderCloseCoordinator();
  return coordinator;
}

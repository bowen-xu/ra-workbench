export class CompositionGate {
  private isComposing = false;

  start(): void {
    this.isComposing = true;
  }

  end(commit: () => void): void {
    this.isComposing = false;
    commit();
  }

  change(commit: () => void): void {
    if (!this.isComposing) commit();
  }
}

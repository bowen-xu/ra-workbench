/** A read-only presentation may own structural review history. It never
 * accepts typing/paste/commands merely because native undo remains enabled. */
export function allowsReadOnlyHistoryTransaction(transaction: {
  readonly docChanged: boolean;
  isUserEvent(event: string): boolean;
}): boolean {
  return !transaction.docChanged || ["input.agent","restore.action","undo","redo"].some(event=>transaction.isUserEvent(event));
}

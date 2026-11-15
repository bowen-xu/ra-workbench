import type {RetainedNoteDocument} from "@research-assistant/workbench-sdk/note-session";
import type {RetainedNoteSession} from "./retainedNoteSession";

// Weak associations belong to the original session objects. A public handle
// retains the same owner while mounted; no timer, writer or HBS model is copied.
const handles = new WeakMap<RetainedNoteSession, RetainedNoteDocument>();
const owners = new WeakMap<RetainedNoteDocument, RetainedNoteSession>();

export function publicNoteSession(session: RetainedNoteSession): RetainedNoteDocument {
  let handle = handles.get(session);
  if (!handle) {
    handle = Object.freeze({
      get snapshot() {return session.snapshot;},
      get editorSessionId() {return session.editorSessionId;},
      get saveStatus() {return session.saveStatus;},
      get saveMessage() {return session.saveMessage;},
    });
    handles.set(session, handle); owners.set(handle, session);
  }
  return handle;
}

/** Only handles produced by the current Core can re-enter its retained editor. */
export function noteSessionOwner(handle: RetainedNoteDocument): RetainedNoteSession {
  const session = owners.get(handle);
  if (!session) throw new Error("NOTE_SESSION_OWNER_UNAVAILABLE：保留编辑器不属于当前Core会话。");
  return session;
}

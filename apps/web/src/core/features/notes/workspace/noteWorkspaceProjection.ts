import type {MutableRefObject} from "react";

/** Presentation only: callbacks follow their current mounted consumer without
 * making new callback identities force a parent/child publication loop. */
export function liveNoteWorkspaceProjection<T extends object>(read: () => T): T {
  const callbacks = new Map<PropertyKey, (...args: unknown[]) => unknown>();
  return new Proxy({} as T, {
    ownKeys: () => Reflect.ownKeys(read()),
    getOwnPropertyDescriptor: () => ({configurable: true, enumerable: true}),
    get: (_target, key) => {
      const value: unknown = Reflect.get(read(), key);
      if (typeof value !== "function") return value;
      let callback = callbacks.get(key);
      if (!callback) {
        callback = (...args) => {
          const current: unknown = Reflect.get(read(), key);
          return typeof current === "function" ? Reflect.apply(current, undefined, args) : undefined;
        };
        callbacks.set(key, callback);
      }
      return callback;
    },
  });
}

/** Compare presentation data, not fresh inline callbacks. Session/DOM/class
 * instances retain identity; only arrays, sets and plain records are traversed. */
export function sameNoteWorkspacePresentation(left: unknown, right: unknown): boolean {
  if (left === right || typeof left === "function" && typeof right === "function") return true;
  if (!left || !right || typeof left !== "object" || typeof right !== "object") return false;
  if (Array.isArray(left) || Array.isArray(right)) return Array.isArray(left) && Array.isArray(right)
    && left.length === right.length && left.every((item, index) => sameNoteWorkspacePresentation(item, right[index]));
  if (left instanceof Set || right instanceof Set) return left instanceof Set && right instanceof Set
    && left.size === right.size && [...left].every(item => right.has(item));
  if (Object.getPrototypeOf(left) !== Object.prototype || Object.getPrototypeOf(right) !== Object.prototype) return false;
  const keys = Reflect.ownKeys(left);
  return keys.length === Reflect.ownKeys(right).length && keys.every(key =>
    Object.hasOwn(right, key) && sameNoteWorkspacePresentation(Reflect.get(left, key), Reflect.get(right, key)));
}

export interface NoteWorkspaceProjection<T extends object> {
  readonly ref: MutableRefObject<T>;
  readonly live: T;
  snapshot: T;
}

export function createNoteWorkspaceProjection<T extends object>(ref: MutableRefObject<T>): NoteWorkspaceProjection<T> {
  return {ref, snapshot: {...ref.current}, live: liveNoteWorkspaceProjection(() => ref.current)};
}

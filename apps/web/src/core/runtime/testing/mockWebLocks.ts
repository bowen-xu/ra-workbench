import { vi } from "vitest";

/** Mock external browser capability; production owners and lock adapters stay real. */
export function mockWebLocks() {
  const held = new Set<string>();
  const request = vi.fn(async (name: string, _options: unknown, callback: (lock: object | null) => Promise<unknown>) => {
    const acquired = !held.has(name); if (acquired) held.add(name);
    try {return await callback(acquired ? {name, mode: "exclusive"} : null);}
    finally {if (acquired) held.delete(name);}
  });
  const original = navigator;
  vi.stubGlobal("navigator", new Proxy(original, {get(target, property) {
    if (property === "locks") return {request};
    const value: unknown = Reflect.get(target, property, target);
    return typeof value === "function" ? value.bind(target) : value;
  }}));
  return {request, held};
}

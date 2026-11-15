/** @vitest-environment happy-dom */
import { act, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { useAgentHostLifecycle } from "./useAgentMountedGuard";

function Probe({ identity, onGuard }: { identity: string; onGuard: (guard: () => boolean) => void }) {
  const lifecycle = useAgentHostLifecycle([identity]);
  useEffect(() => {
    const epoch = lifecycle.current;
    onGuard(() => lifecycle.current === epoch);
  }, [identity, lifecycle, onGuard]);
  return null;
}

it("invalidates host work when its binding changes or unmounts", async () => {
  const guards: Array<() => boolean> = [];
  const onGuard = (guard: () => boolean) => { guards.push(guard); };
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () => root.render(<Probe identity="paper-a" onGuard={onGuard} />));
    const first = guards.at(-1)!;
    expect(first()).toBe(true);
    await act(async () => root.render(<Probe identity="paper-b" onGuard={onGuard} />));
    const second = guards.at(-1)!;
    expect(first()).toBe(false);
    expect(second()).toBe(true);
    await act(async () => root.unmount());
    expect(second()).toBe(false);
  } finally {
    container.remove();
  }
});

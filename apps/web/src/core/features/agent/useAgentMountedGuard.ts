import { useCallback, useEffect, useRef, type MutableRefObject } from "react";

/**
 * Shared lifecycle guard for host-owned Agent controls.
 *
 * Native requests intentionally continue after a React host unmounts, but
 * their UI projection must be ignored. Keeping this ref in one hook prevents
 * each control from inventing a subtly different mounted check.
 */
export function useAgentMountedGuard(): MutableRefObject<boolean> {
  const mounted = useRef(true);
  useEffect(() => {
    // React StrictMode may run an effect cleanup and setup during the same
    // development mount; reset the ref on setup so the second live effect is
    // not mistaken for an unmounted host.
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  return mounted;
}

/**
 * Starts a request epoch and returns a predicate for accepting its result.
 * A later epoch invalidates the previous one; unmounting invalidates all of
 * them through the shared mounted guard.
 */
export function useAgentRequestGuard(): () => () => boolean {
  const mounted = useAgentMountedGuard();
  const epoch = useRef(0);
  return useCallback(() => {
    const request = ++epoch.current;
    return () => mounted.current && request === epoch.current;
  }, [mounted]);
}

/**
 * Invalidates host-owned async work when a bound resource changes or the host
 * unmounts. The resource identity is supplied as a dependency list so writing
 * and note hosts share the same lifecycle rule without copying an epoch effect.
 */
export function useAgentHostLifecycle(dependencies: readonly unknown[]): MutableRefObject<number> {
  const epoch = useRef(0);
  useEffect(() => {
    const current = epoch.current + 1;
    epoch.current = current;
    return () => { if (epoch.current === current) epoch.current += 1; };
  }, dependencies);
  return epoch;
}

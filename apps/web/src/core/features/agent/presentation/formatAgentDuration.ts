/** Compact elapsed time; keep intermediate zero units so the place values remain clear. */
export function formatAgentDuration(seconds: number): string {
  const total = Number.isFinite(seconds) ? Math.max(0, Math.floor(seconds)) : 0;
  const units = [[86400, "日"], [3600, "时"], [60, "分"], [1, "秒"]] as const;
  let remaining = total;
  let result = "";
  for (const [size, label] of units) {
    const value = Math.floor(remaining / size);
    remaining %= size;
    if (value > 0 || result || size === 1) result += `${value}${label}`;
  }
  return result;
}

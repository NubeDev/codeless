// Presentation-only formatters shared across the chat surface
// renderers. Kept in one file because both `ToolCallCard` and
// `LifecycleDivider` reach for the same wall-clock / pretty-JSON
// helpers and the chat module owns the surface.

export function wallClockTime(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => (n < 10 ? `0${n}` : `${n}`);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export function prettyJson(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

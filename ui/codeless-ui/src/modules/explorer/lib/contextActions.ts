import type { ExternalOpenerAdapter } from "@/lib/shell";

export async function copyToClipboard(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Best-effort; ignore in environments without clipboard permission.
  }
}

export function relativePath(rootPath: string, path: string): string {
  if (path === rootPath) return ".";
  if (path.startsWith(`${rootPath}/`)) return path.slice(rootPath.length + 1);
  return path;
}

export async function revealInFinder(
  opener: ExternalOpenerAdapter,
  path: string,
): Promise<void> {
  try {
    await opener.revealPath(path);
  } catch (e) {
    console.error("revealPath failed:", e);
  }
}

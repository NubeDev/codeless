import type { RpcClient } from "@/lib/rpc/client";

import {
  getProvider,
  PROVIDERS,
  providerNeedsKey,
  type ProviderId,
} from "../config";

export type ProviderKeys = Record<ProviderId, string | null>;

export const EMPTY_PROVIDER_KEYS: ProviderKeys = {
  openai: null,
  anthropic: null,
  google: null,
  xai: null,
  cerebras: null,
  groq: null,
  deepseek: null,
  lmstudio: null,
  // Codeless CLI runners are host-managed (Claude Code / Copilot /
  // Codex own their own auth). The keyring entry exists only so the
  // `ProviderKeys` record stays exhaustive over `ProviderId`; it is
  // never populated and `providerNeedsKey` returns false for it.
  codeless: null,
};

export async function getKey(
  rpc: RpcClient,
  provider: ProviderId,
): Promise<string | null> {
  if (!providerNeedsKey(provider)) return null;
  try {
    const v = await rpc.call("secrets_get", {
      provider: getProvider(provider).keyringAccount,
    });
    return v && v.length > 0 ? v : null;
  } catch {
    return null;
  }
}

export async function setKey(
  rpc: RpcClient,
  provider: ProviderId,
  key: string,
): Promise<void> {
  if (!providerNeedsKey(provider)) {
    throw new Error(`${provider} does not use an API key`);
  }
  const trimmed = key.trim();
  if (!trimmed) throw new Error("API key is empty");
  await rpc.call("secrets_set", {
    provider: getProvider(provider).keyringAccount,
    value: trimmed,
  });
}

export async function clearKey(
  rpc: RpcClient,
  provider: ProviderId,
): Promise<void> {
  if (!providerNeedsKey(provider)) return;
  try {
    await rpc.call("secrets_rm", {
      provider: getProvider(provider).keyringAccount,
    });
  } catch {
    // already absent — fine
  }
}

export async function getAllKeys(rpc: RpcClient): Promise<ProviderKeys> {
  const out = { ...EMPTY_PROVIDER_KEYS };
  const need = PROVIDERS.filter((p) => providerNeedsKey(p.id));
  const entries = await Promise.all(
    need.map(async (p) => [p.id, await getKey(rpc, p.id)] as const),
  );
  for (const [id, v] of entries) out[id] = v;
  return out;
}

export function hasAnyKey(keys: ProviderKeys): boolean {
  return PROVIDERS.some((p) => providerNeedsKey(p.id) && !!keys[p.id]);
}

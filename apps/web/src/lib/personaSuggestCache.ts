import { api } from "@/lib/api";
import type { PersonaSuggestion } from "@templar/types";

interface CacheEntry {
  data: PersonaSuggestion[];
  context_hash: string;
  persona_hash: string;
}

// Module-level — persists across component remounts within the same browser tab.
// Dies on page refresh (correct — refresh = clean slate).
let cache: CacheEntry | null = null;
let inFlight: Promise<CacheEntry | null> | null = null;

// Dismissed names survive remount, cleared on page refresh.
const dismissed = new Set<string>();

export function getCachedSuggestions(): PersonaSuggestion[] {
  return cache?.data ?? [];
}

export function isDismissed(name: string): boolean {
  return dismissed.has(name);
}

export function dismissSuggestion(name: string): void {
  dismissed.add(name);
}

export function dismissAll(names: string[]): void {
  names.forEach((n) => dismissed.add(n));
}

/** Call after saving a persona — forces a re-check on next page visit. */
export function invalidateCache(): void {
  cache = null;
  // Preserve `dismissed` — user dismissed for a reason.
}

/**
 * Prefetch persona suggestions and return a Promise<void> that resolves when the caller
 * has initial data to render:
 *
 * - Warm cache:  resolves immediately (stale data served via onUpdate synchronously).
 *               Background revalidation fires silently without blocking.
 * - Cold cache:  resolves when the API call settles
 *               (DB hit ~5ms after first use; LLM ~2-5s on very first call only).
 *
 * Multiple concurrent calls share the same in-flight request — no duplicate LLM calls.
 */
export function prefetchSuggestions(
  userId: string,
  onUpdate: (suggestions: PersonaSuggestion[]) => void,
): Promise<void> {
  if (cache) {
    // Warm: serve stale data right now, resolve the caller's promise immediately.
    onUpdate(cache.data);

    // Background revalidation — does NOT block the returned promise.
    const rev = inFlight ?? _startRequest(userId, cache.context_hash, cache.persona_hash);
    rev.then((result) => {
      if (result && result !== cache) onUpdate(result.data);
    });

    return Promise.resolve();
  }

  // Cold: caller must wait for the first fetch to settle.
  if (!inFlight) _startRequest(userId, undefined, undefined);

  // _startRequest assigned inFlight; attach onUpdate + return a void promise.
  return inFlight!.then((result) => {
    if (result) onUpdate(result.data);
  });
}

function _startRequest(
  userId: string,
  ctxHash: string | undefined,
  personaHash: string | undefined,
): Promise<CacheEntry | null> {
  const p: Promise<CacheEntry | null> = api
    .suggestPersonas(userId, ctxHash, personaHash)
    .then((result): CacheEntry | null => {
      if (result == null) return cache; // 204: cache still valid
      const entry: CacheEntry = {
        data: result.suggestions,
        context_hash: result.context_hash,
        persona_hash: result.persona_hash,
      };
      cache = entry;
      return entry;
    })
    .catch((err) => {
      console.warn("[persona-suggest] fetch silently failed:", err);
      return cache;
    })
    .finally(() => {
      if (inFlight === p) inFlight = null;
    });
  inFlight = p;
  return p;
}

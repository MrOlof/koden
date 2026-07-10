// Shared Librarian model-selection helpers, used by both the onboarding wizard and
// the Settings "Brain" tab. The Librarian does light background work, so it
// defaults to the cheapest curated model of the chosen provider; rates ($/million
// tokens) come from the same MODEL_PRICING the main AI uses so the spend meter stays
// accurate, and local providers are free.
import {
  LMSTUDIO_DEFAULT_BASE_URL,
  MLX_DEFAULT_BASE_URL,
  MODEL_PRICING,
  MODELS,
  OLLAMA_DEFAULT_BASE_URL,
  type ProviderId,
} from "@/modules/ai/config";

export const LOCAL_LIB_PROVIDERS: readonly ProviderId[] = [
  "ollama",
  "lmstudio",
  "mlx",
];

/** A real, usable model id (not a free-form "-custom" placeholder). */
export const isCuratedModelId = (id: string): boolean =>
  !id.endsWith("-custom");

export function isLocalLibProvider(p: ProviderId): boolean {
  return (LOCAL_LIB_PROVIDERS as readonly string[]).includes(p);
}

export function libLocalBaseUrl(p: ProviderId): string {
  if (p === "lmstudio") return LMSTUDIO_DEFAULT_BASE_URL;
  if (p === "mlx") return MLX_DEFAULT_BASE_URL;
  return OLLAMA_DEFAULT_BASE_URL;
}

/** $/million-tokens rates for a provider+model. Local = free; an unknown cloud
 *  model falls back to a conservative tier so the budget can never under-count. */
export function libRates(
  provider: ProviderId,
  model: string,
): { inRate: number; outRate: number } {
  if (isLocalLibProvider(provider)) return { inRate: 0, outRate: 0 };
  const p = MODEL_PRICING[model];
  return p
    ? { inRate: p.input, outRate: p.output }
    : { inRate: 5, outRate: 25 };
}

/** Cheapest curated model for a provider (by input+output $/Mtok), or its first
 *  curated model if none are priced. "" when the provider has no curated model. */
export function cheapestLibModel(provider: ProviderId): string {
  const priced = MODELS.filter(
    (m) => m.provider === provider && isCuratedModelId(m.id),
  )
    .flatMap((m) => {
      const p = MODEL_PRICING[m.id];
      return p ? [{ id: m.id, sum: p.input + p.output }] : [];
    })
    .sort((a, b) => a.sum - b.sum);
  if (priced[0]) return priced[0].id;
  return (
    MODELS.find((m) => m.provider === provider && isCuratedModelId(m.id))?.id ??
    ""
  );
}

/** Curated (selectable) models for a cloud provider. */
export function libCloudModels(provider: ProviderId) {
  return MODELS.filter(
    (m) => m.provider === provider && isCuratedModelId(m.id),
  );
}

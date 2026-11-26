import { isModelId, type AiModelOption } from './ai-model-catalog';
import { AI_PROVIDERS, type AiProvider, type AiProviderId } from './ai-types';

export interface AiProviderPreferences {
  enabled: boolean;
  modelOrder: string[];
  disabledModels: string[];
  customModels: string[];
}

export interface AiAgentPreferences {
  version: 1;
  providerOrder: AiProviderId[];
  providers: Record<AiProviderId, AiProviderPreferences>;
}

const PROVIDER_IDS = AI_PROVIDERS.map(({ id }) => id);

function isProviderId(value: unknown): value is AiProviderId {
  return PROVIDER_IDS.includes(value as AiProviderId);
}

function uniqueModelIds(value: unknown): string[] {
  // 偏好是累计的本地记录，不能套用单次 CLI 目录的数量上限而丢弃整个列表。
  if (!Array.isArray(value)) return [];
  return [...new Set(value.filter(isModelId))];
}

/** 新增 Agent 时自动补到末尾，旧配置无需迁移版本也能得到完整顺序。 */
export function normalizeProviderOrder(value: unknown): AiProviderId[] {
  const stored = Array.isArray(value) ? [...new Set(value.filter(isProviderId))] : [];
  return [...stored, ...PROVIDER_IDS.filter((id) => !stored.includes(id))];
}

export function createAiAgentPreferences(
  customModels: Partial<Record<AiProviderId, string[]>> = {}
): AiAgentPreferences {
  const providers = {} as Record<AiProviderId, AiProviderPreferences>;
  for (const id of PROVIDER_IDS) {
    providers[id] = {
      enabled: true,
      modelOrder: [],
      disabledModels: [],
      customModels: uniqueModelIds(customModels[id]),
    };
  }
  return {
    version: 1,
    providerOrder: [...PROVIDER_IDS],
    providers,
  };
}

/** 配置损坏时只丢弃无效字段；Agent 与合法模型 ID 始终保留可恢复的默认值。 */
export function parseAiAgentPreferences(
  value: unknown,
  legacyCustomModels: Partial<Record<AiProviderId, string[]>> = {}
): AiAgentPreferences {
  const fallback = createAiAgentPreferences(legacyCustomModels);
  if (!value || typeof value !== 'object') return fallback;
  const source = value as Partial<AiAgentPreferences>;
  const providers =
    source.providers && typeof source.providers === 'object' ? source.providers : {};
  const normalizedProviders = {} as Record<AiProviderId, AiProviderPreferences>;
  for (const id of PROVIDER_IDS) {
    const candidate = (providers as Partial<Record<AiProviderId, Partial<AiProviderPreferences>>>)[
      id
    ];
    normalizedProviders[id] = {
      enabled: typeof candidate?.enabled === 'boolean' ? candidate.enabled : true,
      modelOrder: uniqueModelIds(candidate?.modelOrder),
      disabledModels: uniqueModelIds(candidate?.disabledModels),
      customModels: uniqueModelIds(candidate?.customModels ?? legacyCustomModels[id]),
    };
  }

  return {
    version: 1,
    providerOrder: normalizeProviderOrder(source.providerOrder),
    providers: normalizedProviders,
  };
}

/** UI 只消费启用后的有序 Agent；默认浏览项自然就是数组第一项。 */
export function enabledProviders(preferences: AiAgentPreferences): AiProvider[] {
  return preferences.providerOrder
    .filter((id) => preferences.providers[id].enabled)
    .map((id) => AI_PROVIDERS.find((provider) => provider.id === id))
    .filter((provider): provider is AiProvider => provider !== undefined);
}

/** 已保存顺序优先，目录新出现的模型按 Agent 原顺序追加。 */
export function applyModelPreferences(
  models: readonly AiModelOption[],
  preferences: AiProviderPreferences,
  includeDisabled = false
): AiModelOption[] {
  const rank = new Map(preferences.modelOrder.map((id, index) => [id, index]));
  const original = new Map(models.map((model, index) => [model.id, index]));
  return models
    .filter((model) => includeDisabled || !preferences.disabledModels.includes(model.id))
    .sort((left, right) => {
      const leftRank = rank.get(left.id);
      const rightRank = rank.get(right.id);
      if (leftRank !== undefined && rightRank !== undefined) return leftRank - rightRank;
      if (leftRank !== undefined) return -1;
      if (rightRank !== undefined) return 1;
      return (original.get(left.id) ?? 0) - (original.get(right.id) ?? 0);
    });
}

export function moveId(ids: readonly string[], id: string, direction: -1 | 1): string[] {
  const next = [...ids];
  const from = next.indexOf(id);
  const to = from + direction;
  if (from < 0 || to < 0 || to >= next.length) return next;
  [next[from], next[to]] = [next[to], next[from]];
  return next;
}

/** 只调整本轮目录可见项；暂时缺失的模型保留原位置，重新出现后不会丢失历史顺序。 */
export function moveVisibleId(
  storedIds: readonly string[],
  visibleIds: readonly string[],
  id: string,
  direction: -1 | 1
): string[] {
  const reordered = moveId(visibleIds, id, direction);
  const visible = new Set(visibleIds);
  let nextVisible = 0;
  const merged = storedIds.map((stored) =>
    visible.has(stored) ? (reordered[nextVisible++] ?? stored) : stored
  );
  return [...merged, ...reordered.slice(nextVisible)];
}

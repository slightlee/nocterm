import {
  createAiAgentPreferences,
  moveId,
  moveVisibleId,
  parseAiAgentPreferences,
  type AiAgentPreferences,
} from '../model/ai-agent-preferences';
import { isModelId, readCustomModels } from '../model/ai-model-catalog';
import { AI_PROVIDERS, type AiProviderId } from '../model/ai-types';

const STORAGE_KEY = 'nocterm.ai.agent-preferences.v1';
let snapshot: AiAgentPreferences | undefined;
const listeners = new Set<() => void>();

function localStorageOrUndefined(): Storage | undefined {
  try {
    return typeof window === 'undefined' ? undefined : window.localStorage;
  } catch {
    return undefined;
  }
}

function loadPreferences(): AiAgentPreferences {
  const storage = localStorageOrUndefined();
  if (!storage) return createAiAgentPreferences();
  const legacyCustomModels = Object.fromEntries(
    AI_PROVIDERS.map(({ id }) => {
      try {
        return [id, readCustomModels(storage, id)];
      } catch {
        console.warn('无法读取旧版自定义模型列表，原始数据仍保留。');
        return [id, []];
      }
    })
  );
  try {
    const raw = storage.getItem(STORAGE_KEY);
    return raw === null
      ? createAiAgentPreferences(legacyCustomModels)
      : parseAiAgentPreferences(JSON.parse(raw), legacyCustomModels);
  } catch {
    console.warn('无法恢复智能体偏好，将使用默认设置。');
    return createAiAgentPreferences(legacyCustomModels);
  }
}

export function getAiAgentPreferences(): AiAgentPreferences {
  snapshot ??= loadPreferences();
  return snapshot;
}

export function subscribeAiAgentPreferences(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** 先持久化再发布，磁盘失败时所有已挂载视图继续使用同一份旧快照。 */
function updatePreferences(update: (current: AiAgentPreferences) => AiAgentPreferences) {
  const next = update(getAiAgentPreferences());
  const storage = localStorageOrUndefined();
  if (!storage) throw new Error('本地存储不可用');
  storage.setItem(STORAGE_KEY, JSON.stringify(next));
  snapshot = next;
  listeners.forEach((listener) => listener());
}

export function setAiProviderEnabled(provider: AiProviderId, enabled: boolean) {
  updatePreferences((current) => ({
    ...current,
    providers: {
      ...current.providers,
      [provider]: { ...current.providers[provider], enabled },
    },
  }));
}

export function moveAiProvider(provider: AiProviderId, direction: -1 | 1) {
  updatePreferences((current) => ({
    ...current,
    providerOrder: moveId(current.providerOrder, provider, direction) as AiProviderId[],
  }));
}

export function setAiModelEnabled(provider: AiProviderId, model: string, enabled: boolean) {
  if (!isModelId(model)) throw new Error('模型 ID 无效');
  updatePreferences((current) => {
    const settings = current.providers[provider];
    return {
      ...current,
      providers: {
        ...current.providers,
        [provider]: {
          ...settings,
          disabledModels: enabled
            ? settings.disabledModels.filter((id) => id !== model)
            : [...new Set([...settings.disabledModels, model])],
        },
      },
    };
  });
}

export function moveAiModel(
  provider: AiProviderId,
  visibleOrder: readonly string[],
  model: string,
  direction: -1 | 1
) {
  updatePreferences((current) => {
    const settings = current.providers[provider];
    return {
      ...current,
      providers: {
        ...current.providers,
        [provider]: {
          ...settings,
          modelOrder: moveVisibleId(settings.modelOrder, visibleOrder, model, direction),
        },
      },
    };
  });
}

export function addAiCustomModel(provider: AiProviderId, model: string) {
  if (!isModelId(model)) throw new Error('请输入有效的模型 ID');
  updatePreferences((current) => {
    const settings = current.providers[provider];
    return {
      ...current,
      providers: {
        ...current.providers,
        [provider]: {
          ...settings,
          customModels: [...new Set([...settings.customModels, model])],
          disabledModels: settings.disabledModels.filter((id) => id !== model),
        },
      },
    };
  });
}

export function removeAiCustomModel(provider: AiProviderId, model: string) {
  updatePreferences((current) => {
    const settings = current.providers[provider];
    return {
      ...current,
      providers: {
        ...current.providers,
        [provider]: {
          ...settings,
          customModels: settings.customModels.filter((id) => id !== model),
          modelOrder: settings.modelOrder.filter((id) => id !== model),
          disabledModels: settings.disabledModels.filter((id) => id !== model),
        },
      },
    };
  });
}

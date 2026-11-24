import { useCallback, useSyncExternalStore } from 'react';

import {
  getModelCatalogSnapshot,
  subscribeModelCatalog,
  type CatalogSnapshot,
} from '../api/ai-model-catalog';
import type { AiProviderId } from '../model/ai-types';

const EMPTY_SNAPSHOT: CatalogSnapshot = {};

/** 只订阅展示数据；探测仍由面板和菜单按需发起，重绘不会增加 CLI 请求。 */
export function useAiModelCatalog(provider: AiProviderId): CatalogSnapshot {
  const subscribe = useCallback(
    (listener: () => void) => subscribeModelCatalog(provider, listener),
    [provider]
  );
  const getSnapshot = useCallback(() => getModelCatalogSnapshot(provider), [provider]);
  return useSyncExternalStore(subscribe, getSnapshot, () => EMPTY_SNAPSHOT);
}

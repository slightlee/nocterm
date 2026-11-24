import { invoke } from '@tauri-apps/api/core';

import { parseModelCatalog, type AiModelOption } from '../model/ai-model-catalog';
import type { AiProviderId } from '../model/ai-types';

const FRESH_MS = 5 * 60_000;
const RETRY_MS = 30_000;

export interface CatalogSnapshot {
  /** undefined 表示尚无成功结果；空数组是 CLI 正常返回的空目录。 */
  models?: AiModelOption[];
  error?: 'unavailable' | 'not-installed';
}

interface CatalogCache {
  models: AiModelOption[];
}

interface CatalogEntry {
  snapshot: CatalogSnapshot;
  refreshAfter: number;
  pending?: Promise<CatalogSnapshot>;
  listeners: Set<() => void>;
}

const entries = new Map<AiProviderId, CatalogEntry>();

function storageKey(provider: AiProviderId) {
  return 'nocterm.ai.discovered-models.' + provider;
}

function getEntry(provider: AiProviderId): CatalogEntry {
  const existing = entries.get(provider);
  if (existing) return existing;

  const entry: CatalogEntry = { snapshot: {}, refreshAfter: 0, listeners: new Set() };
  try {
    const raw = window.localStorage.getItem(storageKey(provider));
    if (raw !== null) {
      const parsed: unknown = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        entry.snapshot = { models: parseModelCatalog(parsed) };
      } else if (parsed && typeof parsed === 'object') {
        const cache = parsed as Partial<CatalogCache>;
        entry.snapshot = {
          models: parseModelCatalog(cache.models),
        };
      }
    }
  } catch {
    console.warn('无法恢复本机模型目录缓存，将重新读取。');
  }
  entries.set(provider, entry);
  return entry;
}

/** 菜单第一帧直接使用上次成功的模型 ID。跨启动只恢复展示数据，首次使用会重新验证。 */
export function getModelCatalogSnapshot(provider: AiProviderId): CatalogSnapshot {
  return getEntry(provider).snapshot;
}

/** 每个 Agent 独立通知；取消订阅只释放视图，不取消其他视图共用的探测。 */
export function subscribeModelCatalog(provider: AiProviderId, listener: () => void): () => void {
  const { listeners } = getEntry(provider);
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function persistModels(provider: AiProviderId, models?: AiModelOption[]) {
  try {
    if (models === undefined) window.localStorage.removeItem(storageKey(provider));
    else window.localStorage.setItem(storageKey(provider), JSON.stringify({ models }));
  } catch {
    // 磁盘缓存不可用只影响下次启动，本次仍使用内存中的权威探测结果。
    console.warn('无法写入本机模型目录缓存。');
  }
}

/** 面板预取和菜单共用同一请求。按完成时间计时，避免慢探测或短暂故障导致重复启动 CLI。 */
export function refreshModelCatalog(provider: AiProviderId): Promise<CatalogSnapshot> {
  const entry = getEntry(provider);
  if (entry.pending) return entry.pending;
  // 兼容性由同一次后端探测确定；未知版本和失败结果也必须遵守冷却时间。
  if (Date.now() < entry.refreshAfter) return Promise.resolve(entry.snapshot);
  entry.pending = invoke<unknown>('ai_provider_models', { provider })
    .then((raw) => {
      const models = parseModelCatalog(raw);
      persistModels(provider, models);
      entry.snapshot = { models };
      entry.refreshAfter = Date.now() + FRESH_MS;
      return entry.snapshot;
    })
    .catch((reason: unknown) => {
      if (reason === 'AI_PROVIDER_NOT_INSTALLED') {
        persistModels(provider, undefined);
        entry.snapshot = { error: 'not-installed' };
      } else {
        entry.snapshot = {
          models: entry.snapshot.models,
          error: 'unavailable',
        };
      }
      entry.refreshAfter = Date.now() + RETRY_MS;
      return entry.snapshot;
    })
    .finally(() => {
      entry.pending = undefined;
      // 成功、失败和卸载统一发布完整快照，工具栏与菜单不会各保留一份旧状态。
      entry.listeners.forEach((listener) => listener());
    });
  return entry.pending;
}

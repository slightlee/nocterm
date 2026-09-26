import { useEffect, useRef } from 'react';

import {
  deleteSavedAiConversation,
  listAiConversations,
  saveAiConversation,
} from '../api/ai-client';
import {
  conversationToSnapshot,
  diffConversations,
  snapshotToConversation,
} from '../model/ai-persistence';
import { useAiStore } from '../model/ai-store';

/** 差异写入的合并窗口：避免一轮对话触发多次消息变更时反复整会话落盘。 */
const SAVE_DEBOUNCE_MS = 400;

interface AiPersistenceOptions {
  /** 持久化失败时上报 UI；读失败允许降级为纯内存，写失败需要用户感知。 */
  onError: (message: string) => void;
}

/**
 * 把 AI 会话历史接入 SQLite 持久化：
 * - 挂载后读一次全量历史并 hydrate 进 store；
 * - 之后的 store 变更按差异写盘（新增/变化整体覆盖，删除同步删除）；
 * - 浏览器预览没有 IPC，保持纯内存行为，功能不受影响。
 */
export function useAiPersistence({ onError }: AiPersistenceOptions) {
  const hydratedRef = useRef(false);

  useEffect(() => {
    if (!isDesktopRuntime()) return;
    let cancelled = false;
    listAiConversations()
      .then((snapshots) => {
        if (cancelled) return;
        useAiStore.getState().hydrate(snapshots.map(snapshotToConversation));
        // 先完成恢复再开启写回，避免把初始空会话误当成“全部删除再新建”。
        hydratedRef.current = true;
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        hydratedRef.current = true;
        onError(
          error instanceof Error ? error.message : '读取 AI 会话历史失败，本次会话仅保存在内存中。'
        );
      });
    return () => {
      cancelled = true;
    };
  }, [onError]);

  useEffect(() => {
    if (!isDesktopRuntime()) return;
    // 订阅从挂载即开启，但在 hydrate 完成前只累积基线、不发起写盘。
    let prevConversations = useAiStore.getState().conversations;
    let saveTimer: number | null = null;
    let pendingChanged: ReturnType<typeof diffConversations>['changed'] = [];
    const pendingDeleteIds = new Set<string>();

    const flush = () => {
      if (saveTimer !== null) {
        window.clearTimeout(saveTimer);
        saveTimer = null;
      }
      const changed = pendingChanged;
      const deleted = [...pendingDeleteIds];
      pendingChanged = [];
      pendingDeleteIds.clear();
      const writes = changed
        .map(conversationToSnapshot)
        .map((snapshot) => saveAiConversation(snapshot));
      const removals = deleted.map((id) => deleteSavedAiConversation(id));
      // 任一写盘失败都上报，但不中断其余写入：单会话失败不应影响其他历史保存。
      void Promise.all([...writes, ...removals]).catch((error: unknown) => {
        onError(error instanceof Error ? error.message : '保存 AI 会话历史失败。');
      });
    };

    const unsubscribe = useAiStore.subscribe((state) => {
      if (!hydratedRef.current) {
        // hydrate 本身就会整体替换 conversations，这里跟随更新基线即可。
        prevConversations = state.conversations;
        return;
      }
      const { changed, deletedIds } = diffConversations(prevConversations, state.conversations);
      prevConversations = state.conversations;
      if (changed.length > 0) {
        // 合并窗口内同一会话只保留最新快照，写盘量与会话数而非消息数相关。
        const byId = new Map(pendingChanged.map((conversation) => [conversation.id, conversation]));
        changed.forEach((conversation) => byId.set(conversation.id, conversation));
        pendingChanged = [...byId.values()];
        if (saveTimer === null) {
          saveTimer = window.setTimeout(flush, SAVE_DEBOUNCE_MS);
        }
      }
      deletedIds.forEach((id) => pendingDeleteIds.add(id));
      if (deletedIds.length > 0) flush();
    });

    return () => {
      unsubscribe();
      // 卸载前把尚未落盘的变更刷出去，防止关闭窗口丢失最后一次修改。
      flush();
    };
  }, [onError]);
}

function isDesktopRuntime(): boolean {
  // 延迟引用避免在模块加载期产生副作用；判断逻辑与 shared/lib/tauri-runtime 保持一致。
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

import { useState } from 'react';

import { conversationTargetKey, type AiConversation } from '../model/ai-store';
import styles from './AiHistoryMenu.module.css';

interface AiHistoryMenuProps {
  conversations: AiConversation[];
  activeConversationId: string | null;
  currentTargetKey: string | null;
  runningConversationIds: string[];
  canClear: boolean;
  onClear: () => void;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
}

/** 历史菜单只展示会话并转发意图，Provider 重置和 Store 更新由面板编排。 */
export function AiHistoryMenu({
  conversations,
  activeConversationId,
  currentTargetKey,
  runningConversationIds,
  canClear,
  onClear,
  onSelect,
  onDelete,
}: AiHistoryMenuProps) {
  const [scope, setScope] = useState<'current' | 'all'>(currentTargetKey ? 'current' : 'all');
  const runningConversationIdSet = new Set(runningConversationIds);
  const visibleConversations = conversations
    .filter(
      (conversation) => scope === 'all' || conversationTargetKey(conversation) === currentTargetKey
    )
    .sort((a, b) => b.updatedAt - a.updatedAt);

  return (
    <div className={styles.historyPanel}>
      <div className={styles.historyHeader}>
        <span>历史会话</span>
        <div className={styles.historyActions}>
          {canClear ? (
            <button
              disabled={Boolean(
                activeConversationId && runningConversationIdSet.has(activeConversationId)
              )}
              onClick={onClear}
              type="button"
            >
              清空
            </button>
          ) : null}
        </div>
      </div>
      <div className={styles.historyScope} role="group" aria-label="历史范围">
        <button
          aria-pressed={scope === 'current'}
          disabled={!currentTargetKey}
          onClick={() => setScope('current')}
          type="button"
        >
          当前目标
        </button>
        <button aria-pressed={scope === 'all'} onClick={() => setScope('all')} type="button">
          全部
        </button>
      </div>
      <div className={styles.historyList}>
        {visibleConversations.length ? (
          visibleConversations.map((conversation) => (
            <div
              className={`${styles.historyItem} ${conversation.id === activeConversationId ? styles.historyItemActive : ''}`}
              key={conversation.id}
            >
              <button
                className={styles.historySelect}
                onClick={() => onSelect(conversation.id)}
                type="button"
              >
                <span className={styles.historyCopy}>
                  <span>{conversation.title}</span>
                  {scope === 'all' ? (
                    <small>{conversation.target?.label ?? '未绑定历史'}</small>
                  ) : null}
                </span>
              </button>
              <button
                aria-label={`删除会话：${conversation.title}`}
                className={styles.historyDelete}
                disabled={runningConversationIdSet.has(conversation.id)}
                onClick={() => onDelete(conversation.id)}
                title="删除会话"
                type="button"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13M10 11v5M14 11v5" />
                </svg>
              </button>
            </div>
          ))
        ) : (
          <div className={styles.historyEmpty}>暂无会话</div>
        )}
      </div>
    </div>
  );
}

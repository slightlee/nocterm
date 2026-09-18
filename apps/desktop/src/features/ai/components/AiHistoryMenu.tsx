import type { AiConversation } from '../model/ai-store';
import styles from './AiHistoryMenu.module.css';

interface AiHistoryMenuProps {
  conversations: AiConversation[];
  activeConversationId: string;
  disabled: boolean;
  canClear: boolean;
  onClear: () => void;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
}

/** 历史菜单只展示会话并转发意图，Provider 重置和 Store 更新由面板编排。 */
export function AiHistoryMenu({
  conversations,
  activeConversationId,
  disabled,
  canClear,
  onClear,
  onSelect,
  onDelete,
}: AiHistoryMenuProps) {
  return (
    <div className={styles.historyPanel}>
      <div className={styles.historyHeader}>
        <span>历史会话</span>
        <div className={styles.historyActions}>
          {canClear ? (
            <button disabled={disabled} onClick={onClear} type="button">
              清空
            </button>
          ) : null}
        </div>
      </div>
      <div className={styles.historyList}>
        {conversations
          .slice()
          .sort((a, b) => b.updatedAt - a.updatedAt)
          .map((conversation) => (
            <div
              className={`${styles.historyItem} ${conversation.id === activeConversationId ? styles.historyItemActive : ''}`}
              key={conversation.id}
            >
              <button
                className={styles.historySelect}
                disabled={disabled}
                onClick={() => onSelect(conversation.id)}
                type="button"
              >
                <span>{conversation.title}</span>
              </button>
              <button
                aria-label={`删除会话：${conversation.title}`}
                className={styles.historyDelete}
                disabled={disabled}
                onClick={() => onDelete(conversation.id)}
                title="删除会话"
                type="button"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13M10 11v5M14 11v5" />
                </svg>
              </button>
            </div>
          ))}
      </div>
    </div>
  );
}

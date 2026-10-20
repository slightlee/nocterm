import styles from './AiPanelToggle.module.css';

interface AiPanelToggleProps {
  expanded: boolean;
  onToggle: () => void;
}

/** 收起时由会话栏承载，展开后转入 AI 标题栏，视觉位置始终贴近窗口右上角。 */
export function AiPanelToggle({ expanded, onToggle }: AiPanelToggleProps) {
  const label = expanded ? '收起 AI 助手' : '展开 AI 助手';

  return (
    <button
      aria-controls="ai-panel"
      aria-expanded={expanded}
      aria-label={label}
      className={`${styles.toggle} ${expanded ? styles.compact : ''}`}
      data-tooltip={expanded ? label : undefined}
      onClick={onToggle}
      title={expanded ? undefined : label}
      type="button"
    >
      {expanded ? (
        // 复用连接面板的折叠资源，让左右侧栏使用同一套图标语言。
        <span aria-hidden="true" className={styles.panelCollapseIcon} />
      ) : (
        <>
          <svg aria-hidden="true" className={styles.assistantIcon} viewBox="0 0 24 24">
            <path d="M5 4.5h14a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-7l-4.5 3v-3H5a2 2 0 0 1-2-2v-9a2 2 0 0 1 2-2Z" />
            <path d="m7 9 2 2-2 2M12 13h4" />
          </svg>
          <span>AI 助手</span>
        </>
      )}
    </button>
  );
}

import type { TerminalSelectionActionState } from '../model/terminal-selection';
import styles from './SshTerminal.module.css';

interface TerminalSelectionActionProps {
  action: TerminalSelectionActionState | null;
  onAdd: (selection: string) => void;
}

/** 入口贴近用户刚完成的选择，点击只加入 AI 上下文，不直接发送请求。 */
export function TerminalSelectionAction({ action, onAdd }: TerminalSelectionActionProps) {
  if (!action) return null;
  return (
    <button
      aria-label="将终端选区添加到 AI"
      className={`${styles.selectionAction} ${action.compact ? styles.selectionActionCompact : ''}`}
      onClick={() => onAdd(action.selection)}
      style={{ left: action.left, top: action.top }}
      title="添加到 AI 上下文"
      type="button"
    >
      <svg aria-hidden="true" viewBox="0 0 24 24">
        <path d="M5 4.5h14a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-7l-4.5 3v-3H5a2 2 0 0 1-2-2v-9a2 2 0 0 1 2-2Z" />
        <path d="m7 9 2 2-2 2M12 13h4" />
      </svg>
      <span>添加到 AI</span>
    </button>
  );
}

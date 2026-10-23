import type { ChangeEvent, RefObject } from 'react';

import type { AiAttachment } from '../model/ai-attachment';
import styles from './AiComposerControls.module.css';

interface AiAttachmentPickerProps {
  loading: boolean;
  disabled?: boolean;
  fileInputRef: RefObject<HTMLInputElement | null>;
  onChange: (event: ChangeEvent<HTMLInputElement>) => void;
}

interface AiAttachmentChipProps {
  attachment: AiAttachment;
  disabled?: boolean;
  onRemove: () => void;
  onReplace?: () => void;
}

/** 已选附件独立于工具栏展示，文件名变化不会挤压 Provider 和权限控件。 */
export function AiAttachmentChip({
  attachment,
  disabled = false,
  onRemove,
  onReplace,
}: AiAttachmentChipProps) {
  const removeLabel =
    attachment.kind === 'terminal-selection' ? '移除终端选区' : `移除附件：${attachment.name}`;
  const label = (
    <>
      {attachment.kind === 'terminal-selection' ? (
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <rect x="4" y="5" width="16" height="14" rx="2" />
          <path d="m8 10 2 2-2 2M12 14h4" />
        </svg>
      ) : (
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="m9 12 5-5a3 3 0 1 1 4 4l-7 7a5 5 0 0 1-7-7l7-7" />
        </svg>
      )}
      <span>{attachment.name}</span>
    </>
  );

  return (
    <div className={styles.attachmentChip} title={attachment.name}>
      {onReplace ? (
        <button
          className={styles.attachmentName}
          disabled={disabled}
          onClick={onReplace}
          title="更换附件"
          type="button"
        >
          {label}
        </button>
      ) : (
        <span className={`${styles.attachmentName} ${styles.attachmentNameStatic}`}>{label}</span>
      )}
      <button
        aria-label={removeLabel}
        className={styles.attachmentRemove}
        disabled={disabled}
        onClick={onRemove}
        title={removeLabel}
        type="button"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="m7 7 10 10M17 7 7 17" />
        </svg>
      </button>
    </div>
  );
}

/** 文件输入只保留一个实例；已选附件由输入区中的独立 Chip 展示。 */
export function AiAttachmentPicker({
  loading,
  disabled = false,
  fileInputRef,
  onChange,
}: AiAttachmentPickerProps) {
  return (
    <>
      <input
        accept=".log,.txt,.md,.json,.yaml,.yml,.toml,.xml,.csv,.conf,.ini,.sh,.ps1,text/*,application/json"
        aria-label="选择文件"
        className={styles.fileInput}
        disabled={disabled}
        onChange={onChange}
        ref={fileInputRef}
        type="file"
      />
      <button
        aria-label={loading ? '正在读取文件' : '添加文件'}
        className={`${styles.attachmentButton} ${loading ? styles.attachmentButtonLoading : ''}`}
        disabled={disabled || loading}
        onClick={() => fileInputRef.current?.click()}
        title={loading ? '正在读取文件' : disabled ? '当前会话不可编辑' : '添加文件'}
        type="button"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="m9 12 5-5a3 3 0 1 1 4 4l-7 7a5 5 0 0 1-7-7l7-7" />
        </svg>
      </button>
    </>
  );
}

import type { ChangeEvent, RefObject } from 'react';

import type { AiAttachment } from '../model/ai-attachment';
import styles from './AiComposerControls.module.css';

interface AiAttachmentPickerProps {
  loading: boolean;
  fileInputRef: RefObject<HTMLInputElement | null>;
  onChange: (event: ChangeEvent<HTMLInputElement>) => void;
}

interface AiAttachmentChipProps {
  attachment: AiAttachment;
  fileInputRef: RefObject<HTMLInputElement | null>;
  onRemove: () => void;
}

/** 已选附件独立于工具栏展示，文件名变化不会挤压 Provider 和权限控件。 */
export function AiAttachmentChip({ attachment, fileInputRef, onRemove }: AiAttachmentChipProps) {
  return (
    <div className={styles.attachmentChip} title={attachment.name}>
      <button
        className={styles.attachmentName}
        onClick={() => fileInputRef.current?.click()}
        title="更换附件"
        type="button"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="m9 12 5-5a3 3 0 1 1 4 4l-7 7a5 5 0 0 1-7-7l7-7" />
        </svg>
        <span>{attachment.name}</span>
      </button>
      <button
        aria-label={`移除附件：${attachment.name}`}
        className={styles.attachmentRemove}
        onClick={onRemove}
        title="移除附件"
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
export function AiAttachmentPicker({ loading, fileInputRef, onChange }: AiAttachmentPickerProps) {
  return (
    <>
      <input
        accept=".log,.txt,.md,.json,.yaml,.yml,.toml,.xml,.csv,.conf,.ini,.sh,.ps1,text/*,application/json"
        aria-label="选择文件"
        className={styles.fileInput}
        onChange={onChange}
        ref={fileInputRef}
        type="file"
      />
      <button
        aria-label={loading ? '正在读取文件' : '添加文件'}
        className={styles.attachmentButton}
        disabled={loading}
        onClick={() => fileInputRef.current?.click()}
        title={loading ? '正在读取文件' : '添加文件'}
        type="button"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="m9 12 5-5a3 3 0 1 1 4 4l-7 7a5 5 0 0 1-7-7l7-7" />
        </svg>
      </button>
    </>
  );
}

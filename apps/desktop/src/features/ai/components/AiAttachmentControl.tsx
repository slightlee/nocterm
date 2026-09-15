import type { ChangeEvent, RefObject } from 'react';

import type { AiAttachment } from '../model/ai-attachment';
import styles from './AiPanel.module.css';

interface AiAttachmentControlProps {
  attachment: AiAttachment | null;
  loading: boolean;
  fileInputRef: RefObject<HTMLInputElement | null>;
  onChange: (event: ChangeEvent<HTMLInputElement>) => void;
  onRemove: () => void;
}

/** 附件控件封装文件选择入口和单附件替换/移除状态。 */
export function AiAttachmentControl({
  attachment,
  loading,
  fileInputRef,
  onChange,
  onRemove,
}: AiAttachmentControlProps) {
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
      {attachment ? (
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
      ) : (
        <button
          className={styles.attachmentButton}
          disabled={loading}
          onClick={() => fileInputRef.current?.click()}
          title="添加文件"
          type="button"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="m9 12 5-5a3 3 0 1 1 4 4l-7 7a5 5 0 0 1-7-7l7-7" />
          </svg>
          <span>{loading ? '读取中…' : '添加文件'}</span>
        </button>
      )}
    </>
  );
}

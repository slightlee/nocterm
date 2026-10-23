import {
  useState,
  type ChangeEvent,
  type FormEvent,
  type KeyboardEvent,
  type RefObject,
} from 'react';

import type { AiAttachment } from '../model/ai-attachment';
import type { AiCommandPolicy, AiProvider, AiProviderId } from '../model/ai-types';
import { AiAttachmentChip, AiAttachmentPicker } from './AiAttachmentControl';
import { AiCommandPolicySelector } from './AiCommandPolicySelector';
import { AiProviderSelector } from './AiProviderSelector';
import styles from './AiComposer.module.css';

interface AiComposerProps {
  provider: AiProvider;
  providerId: AiProviderId;
  providerAvailable: boolean | undefined;
  providerMenuOpen: boolean;
  running: boolean;
  controlsDisabled: boolean;
  composerDisabled: boolean;
  statusMessage?: string;
  draft: string;
  attachments: readonly AiAttachment[];
  attachmentLoading: boolean;
  targetReady: boolean;
  targetUnavailableMessage: string;
  composerRef: RefObject<HTMLTextAreaElement | null>;
  fileInputRef: RefObject<HTMLInputElement | null>;
  onSubmit: (event: FormEvent) => void;
  onDraftChange: (value: string) => void;
  onComposerKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  onAttachmentChange: (event: ChangeEvent<HTMLInputElement>) => void;
  onAttachmentRemove: (kind: AiAttachment['kind']) => void;
  onProviderMenuToggle: () => void;
  onProviderChange: (provider: AiProviderId) => void;
  commandPolicy: AiCommandPolicy;
  onCommandPolicyChange: (policy: AiCommandPolicy) => void;
  onStop: () => void;
}

/** 输入器只负责采集用户输入；会话切换、附件读取和 Provider 停止由宿主处理。 */
export function AiComposer({
  provider,
  providerId,
  providerAvailable,
  providerMenuOpen,
  running,
  controlsDisabled,
  composerDisabled,
  statusMessage,
  draft,
  attachments,
  attachmentLoading,
  targetReady,
  targetUnavailableMessage,
  composerRef,
  fileInputRef,
  onSubmit,
  onDraftChange,
  onComposerKeyDown,
  onAttachmentChange,
  onAttachmentRemove,
  onProviderMenuToggle,
  onProviderChange,
  commandPolicy,
  onCommandPolicyChange,
  onStop,
}: AiComposerProps) {
  const [commandPolicyMenuOpen, setCommandPolicyMenuOpen] = useState(false);
  return (
    <div className={styles.composerArea}>
      {providerAvailable === false ? (
        <div className={styles.bridgeNotice}>
          <span className={styles.bridgeDot} />
          <span>未安装 {provider.command}</span>
        </div>
      ) : null}
      {statusMessage ? (
        <div className={styles.bridgeNotice} role="status">
          <span className={styles.bridgeDot} />
          <span>{statusMessage}</span>
        </div>
      ) : null}
      <form className={styles.composer} onSubmit={onSubmit}>
        {attachments.length > 0 ? (
          <div className={styles.attachmentRow}>
            {attachments.map((attachment) => (
              <AiAttachmentChip
                attachment={attachment}
                disabled={composerDisabled || controlsDisabled}
                key={attachment.kind}
                onRemove={() => onAttachmentRemove(attachment.kind)}
                onReplace={
                  attachment.kind === 'file' ? () => fileInputRef.current?.click() : undefined
                }
              />
            ))}
          </div>
        ) : null}
        <textarea
          aria-label="输入 AI 请求"
          disabled={composerDisabled}
          onChange={(event) => onDraftChange(event.target.value)}
          onKeyDown={onComposerKeyDown}
          placeholder="输入问题或直接告诉我想做什么…"
          rows={2}
          ref={composerRef}
          value={draft}
        />
        <div className={styles.composerFooter}>
          <div className={styles.composerTools}>
            <AiAttachmentPicker
              disabled={composerDisabled || controlsDisabled}
              fileInputRef={fileInputRef}
              loading={attachmentLoading}
              onChange={onAttachmentChange}
            />
            <AiProviderSelector
              disabled={composerDisabled || controlsDisabled}
              onChange={onProviderChange}
              onToggle={() => {
                setCommandPolicyMenuOpen(false);
                onProviderMenuToggle();
              }}
              open={providerMenuOpen}
              provider={provider}
              providerId={providerId}
            />
            <AiCommandPolicySelector
              disabled={composerDisabled || controlsDisabled}
              onChange={(policy) => {
                onCommandPolicyChange(policy);
                setCommandPolicyMenuOpen(false);
              }}
              onToggle={() => {
                if (providerMenuOpen) onProviderMenuToggle();
                setCommandPolicyMenuOpen((open) => !open);
              }}
              open={commandPolicyMenuOpen}
              policy={commandPolicy}
            />
          </div>
          {running ? (
            <button
              aria-label="停止 AI 会话"
              className={styles.sendButton}
              onClick={onStop}
              title="停止 AI 会话"
              type="button"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <rect x="7" y="7" width="10" height="10" rx="1" />
              </svg>
            </button>
          ) : (
            <button
              aria-label="发送请求"
              className={styles.sendButton}
              disabled={
                composerDisabled ||
                !targetReady ||
                (!draft.trim() && attachments.length === 0) ||
                attachmentLoading
              }
              title={targetReady ? '发送请求' : targetUnavailableMessage}
              type="submit"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="m5 12 14-7-3 14-4-6-7-1Z" />
                <path d="m12 13 4-5" />
              </svg>
            </button>
          )}
        </div>
      </form>
    </div>
  );
}

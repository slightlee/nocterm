import {
  useState,
  type ChangeEvent,
  type FormEvent,
  type KeyboardEvent,
  type RefObject,
} from 'react';

import type { AiAttachment } from '../model/ai-attachment';
import type { AiCommandPolicy, AiProvider, AiProviderId } from '../model/ai-types';
import { AiAttachmentControl } from './AiAttachmentControl';
import { AiCommandPolicySelector } from './AiCommandPolicySelector';
import { AiProviderSelector } from './AiProviderSelector';
import styles from './AiComposer.module.css';

interface AiComposerProps {
  provider: AiProvider;
  providerId: AiProviderId;
  providerAvailable: boolean | undefined;
  providerMenuOpen: boolean;
  runningSessionId: string | null;
  draft: string;
  attachment: AiAttachment | null;
  attachmentLoading: boolean;
  terminalReady: boolean;
  composerRef: RefObject<HTMLTextAreaElement | null>;
  fileInputRef: RefObject<HTMLInputElement | null>;
  onSubmit: (event: FormEvent) => void;
  onDraftChange: (value: string) => void;
  onComposerKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  onAttachmentChange: (event: ChangeEvent<HTMLInputElement>) => void;
  onAttachmentRemove: () => void;
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
  runningSessionId,
  draft,
  attachment,
  attachmentLoading,
  terminalReady,
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
      <form className={styles.composer} onSubmit={onSubmit}>
        <textarea
          aria-label="输入 AI 请求"
          onChange={(event) => onDraftChange(event.target.value)}
          onKeyDown={onComposerKeyDown}
          placeholder="输入问题或直接告诉我想做什么…"
          rows={3}
          ref={composerRef}
          value={draft}
        />
        <div className={styles.composerFooter}>
          <div className={styles.composerTools}>
            <AiProviderSelector
              disabled={Boolean(runningSessionId)}
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
              disabled={Boolean(runningSessionId)}
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
            <AiAttachmentControl
              attachment={attachment}
              fileInputRef={fileInputRef}
              loading={attachmentLoading}
              onChange={onAttachmentChange}
              onRemove={onAttachmentRemove}
            />
          </div>
          {runningSessionId ? (
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
              disabled={!terminalReady || (!draft.trim() && !attachment) || attachmentLoading}
              title={terminalReady ? '发送请求' : '请先连接终端'}
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

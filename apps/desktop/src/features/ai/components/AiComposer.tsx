import {
  useState,
  type ChangeEvent,
  type FormEvent,
  type KeyboardEvent,
  type RefObject,
} from 'react';

import type { AiAttachment } from '../model/ai-attachment';
import {
  AI_PROVIDERS,
  type AiCommandPolicy,
  type AiProvider,
  type AiProviderId,
} from '../model/ai-types';
import styles from './AiPanel.module.css';

const COMMAND_POLICY_OPTIONS: ReadonlyArray<{
  id: AiCommandPolicy;
  label: string;
  description: string;
  recommended?: boolean;
  dangerous?: boolean;
}> = [
  { id: 'deny_all', label: '仅分析', description: '不执行终端操作，仅基于已有上下文回答' },
  { id: 'confirm_each', label: '每次确认', description: '每次访问终端前都需要你确认' },
  {
    id: 'auto_safe',
    label: '变更前确认',
    description: '自动读取，修改系统或未知操作前确认',
    recommended: true,
  },
  {
    id: 'full_access',
    label: '完全访问',
    description: '当前会话内执行终端操作不再确认',
    dangerous: true,
  },
];

function CommandPolicyIcon({ policy, className }: { policy: AiCommandPolicy; className?: string }) {
  if (policy === 'deny_all') {
    return (
      <svg className={className} viewBox="0 0 24 24" aria-hidden="true">
        <path d="M3 3l18 18M10.6 10.7a2 2 0 0 0 2.7 2.7M9.9 4.2A10.8 10.8 0 0 1 21 12a12.8 12.8 0 0 1-3 4.3M6.2 6.2A12.5 12.5 0 0 0 3 12s3.5 6 9 6c.7 0 1.4-.1 2-.3" />
      </svg>
    );
  }
  if (policy === 'confirm_each') {
    return (
      <svg className={className} viewBox="0 0 24 24" aria-hidden="true">
        <path d="M7.5 11V6.5a1.5 1.5 0 0 1 3 0V10M10.5 10V4.5a1.5 1.5 0 0 1 3 0V10M13.5 10V6a1.5 1.5 0 0 1 3 0v5M16.5 11V8.5a1.5 1.5 0 0 1 3 0V14c0 4-2.8 7-7 7h-1c-2.2 0-4.2-1-5.5-2.8L3.3 14a1.7 1.7 0 0 1 2.6-2.1L7.5 14" />
      </svg>
    );
  }
  if (policy === 'auto_safe') {
    return (
      <svg className={className} viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 3 5 6v5c0 4.5 2.8 8.5 7 10 4.2-1.5 7-5.5 7-10V6l-7-3Z" />
        <path d="m9.5 12 1.7 1.7 3.5-3.5" />
      </svg>
    );
  }
  return (
    <svg className={className} viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 3 5 6v5c0 4.5 2.8 8.5 7 10 4.2-1.5 7-5.5 7-10V6l-7-3Z" />
      <path d="M12 8v5M12 16.5v.5" />
    </svg>
  );
}

interface AiComposerProps {
  provider: AiProvider;
  providerId: AiProviderId;
  providerAvailable: boolean | undefined;
  providerMenuOpen: boolean;
  runningSessionId: string | null;
  draft: string;
  attachment: AiAttachment | null;
  attachmentLoading: boolean;
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
  const policyLabel =
    COMMAND_POLICY_OPTIONS.find((option) => option.id === commandPolicy)?.label ?? '变更前确认';
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
          placeholder={`询问 ${provider.name}…`}
          rows={3}
          ref={composerRef}
          value={draft}
        />
        <div className={styles.composerFooter}>
          <div className={styles.composerTools}>
            <div className={styles.providerMenu}>
              <button
                aria-expanded={providerMenuOpen}
                className={`${styles.providerButton} ${styles.composerProviderButton}`}
                disabled={Boolean(runningSessionId)}
                onClick={() => {
                  setCommandPolicyMenuOpen(false);
                  onProviderMenuToggle();
                }}
                title="切换 AI Provider"
                type="button"
              >
                <span className={styles.providerLabel}>
                  <svg className={styles.providerIcon} viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M7 5h10v14H7zM10 9h4M10 13h4" />
                  </svg>
                  <span>{provider.name}</span>
                </span>
                <svg className={styles.providerChevron} viewBox="0 0 24 24" aria-hidden="true">
                  <path d="m7 10 5 5 5-5" />
                </svg>
              </button>
              {providerMenuOpen ? (
                <div className={styles.providerOptions} role="listbox" aria-label="Agent Provider">
                  {AI_PROVIDERS.map((item) => (
                    <button
                      aria-selected={item.id === providerId}
                      className={
                        item.id === providerId ? styles.providerOptionActive : styles.providerOption
                      }
                      key={item.id}
                      disabled={Boolean(runningSessionId)}
                      onClick={() => onProviderChange(item.id)}
                      role="option"
                      type="button"
                    >
                      <span>{item.name}</span>
                      <small>{item.description}</small>
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
            <div className={styles.providerMenu}>
              <button
                aria-expanded={commandPolicyMenuOpen}
                aria-haspopup="listbox"
                className={`${styles.providerButton} ${styles.composerProviderButton}`}
                disabled={Boolean(runningSessionId)}
                onClick={() => {
                  if (providerMenuOpen) onProviderMenuToggle();
                  setCommandPolicyMenuOpen((open) => !open);
                }}
                title="设置终端命令执行权限"
                type="button"
              >
                <span className={styles.providerLabel}>
                  <CommandPolicyIcon
                    className={`${styles.providerIcon} ${
                      commandPolicy === 'full_access' ? styles.commandPolicyButtonDanger : ''
                    }`}
                    policy={commandPolicy}
                  />
                  <span>{policyLabel}</span>
                </span>
                <svg className={styles.providerChevron} viewBox="0 0 24 24" aria-hidden="true">
                  <path d="m7 10 5 5 5-5" />
                </svg>
              </button>
              {commandPolicyMenuOpen ? (
                <div
                  className={`${styles.providerOptions} ${styles.commandPolicyOptions}`}
                  role="listbox"
                  aria-label="执行权限"
                >
                  {COMMAND_POLICY_OPTIONS.map((option) => (
                    <button
                      aria-selected={commandPolicy === option.id}
                      className={`${
                        commandPolicy === option.id
                          ? styles.providerOptionActive
                          : styles.providerOption
                      } ${styles.commandPolicyOption} ${option.dangerous ? styles.commandPolicyOptionDanger : ''}`}
                      disabled={Boolean(runningSessionId)}
                      key={option.id}
                      onClick={() => {
                        onCommandPolicyChange(option.id);
                        setCommandPolicyMenuOpen(false);
                      }}
                      role="option"
                      type="button"
                    >
                      <span className={styles.commandPolicyIcon}>
                        <CommandPolicyIcon policy={option.id} />
                      </span>
                      <span className={styles.commandPolicyCopy}>
                        <span>
                          {option.label}
                          {option.recommended ? <em>推荐</em> : null}
                        </span>
                        <small>{option.description}</small>
                      </span>
                      {commandPolicy === option.id ? (
                        <svg
                          className={styles.commandPolicySelected}
                          viewBox="0 0 24 24"
                          aria-hidden="true"
                        >
                          <path d="m6 12 4 4 8-8" />
                        </svg>
                      ) : null}
                    </button>
                  ))}
                  <div className={styles.commandPolicyHint} role="note">
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path d="M12 3 5 6v5c0 4.5 2.8 8.5 7 10 4.2-1.5 7-5.5 7-10V6l-7-3Z" />
                    </svg>
                    <span>完全访问仅对当前会话生效；新会话恢复为“变更前确认”。</span>
                  </div>
                </div>
              ) : null}
            </div>
            <input
              accept=".log,.txt,.md,.json,.yaml,.yml,.toml,.xml,.csv,.conf,.ini,.sh,.ps1,text/*,application/json"
              aria-label="选择文件"
              className={styles.fileInput}
              onChange={onAttachmentChange}
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
                  onClick={onAttachmentRemove}
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
                disabled={attachmentLoading}
                onClick={() => fileInputRef.current?.click()}
                title="添加文件"
                type="button"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d="m9 12 5-5a3 3 0 1 1 4 4l-7 7a5 5 0 0 1-7-7l7-7" />
                </svg>
                <span>{attachmentLoading ? '读取中…' : '添加文件'}</span>
              </button>
            )}
          </div>
          {runningSessionId ? (
            <button
              aria-label="停止 AI 会话"
              className={`${styles.sendButton} ${styles.stopButton}`}
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
              disabled={(!draft.trim() && !attachment) || attachmentLoading}
              title="发送请求"
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

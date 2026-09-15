import type { AiCommandPolicy } from '../model/ai-types';
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

interface AiCommandPolicySelectorProps {
  policy: AiCommandPolicy;
  open: boolean;
  disabled: boolean;
  onToggle: () => void;
  onChange: (policy: AiCommandPolicy) => void;
}

/** 权限选择器保持四级策略的单调顺序，并突出当前会话的高风险模式。 */
export function AiCommandPolicySelector({
  policy,
  open,
  disabled,
  onToggle,
  onChange,
}: AiCommandPolicySelectorProps) {
  const policyLabel =
    COMMAND_POLICY_OPTIONS.find((option) => option.id === policy)?.label ?? '变更前确认';

  return (
    <div className={styles.providerMenu}>
      <button
        aria-expanded={open}
        aria-haspopup="listbox"
        className={`${styles.providerButton} ${styles.composerProviderButton}`}
        disabled={disabled}
        onClick={onToggle}
        title="设置终端命令执行权限"
        type="button"
      >
        <span className={styles.providerLabel}>
          <CommandPolicyIcon
            className={`${styles.providerIcon} ${
              policy === 'full_access' ? styles.commandPolicyButtonDanger : ''
            }`}
            policy={policy}
          />
          <span>{policyLabel}</span>
        </span>
        <svg className={styles.providerChevron} viewBox="0 0 24 24" aria-hidden="true">
          <path d="m7 10 5 5 5-5" />
        </svg>
      </button>
      {open ? (
        <div
          className={`${styles.providerOptions} ${styles.commandPolicyOptions}`}
          role="listbox"
          aria-label="执行权限"
        >
          {COMMAND_POLICY_OPTIONS.map((option) => (
            <button
              aria-selected={policy === option.id}
              className={`${
                policy === option.id ? styles.providerOptionActive : styles.providerOption
              } ${styles.commandPolicyOption} ${option.dangerous ? styles.commandPolicyOptionDanger : ''}`}
              disabled={disabled}
              key={option.id}
              onClick={() => onChange(option.id)}
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
              {policy === option.id ? (
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
  );
}

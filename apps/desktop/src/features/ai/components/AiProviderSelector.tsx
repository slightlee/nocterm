import { AI_PROVIDERS, type AiProvider, type AiProviderId } from '../model/ai-types';
import styles from './AiComposerControls.module.css';

interface AiProviderSelectorProps {
  provider: AiProvider;
  providerId: AiProviderId;
  open: boolean;
  disabled: boolean;
  onToggle: () => void;
  onChange: (provider: AiProviderId) => void;
}

/** Provider 选择器只呈现已注册 Adapter，不参与会话启动或可用性探测。 */
export function AiProviderSelector({
  provider,
  providerId,
  open,
  disabled,
  onToggle,
  onChange,
}: AiProviderSelectorProps) {
  return (
    <div className={styles.providerMenu}>
      <button
        aria-expanded={open}
        className={`${styles.providerButton} ${styles.composerProviderButton}`}
        disabled={disabled}
        onClick={onToggle}
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
      {open ? (
        <div className={styles.providerOptions} role="listbox" aria-label="Agent Provider">
          {AI_PROVIDERS.map((item) => (
            <button
              aria-selected={item.id === providerId}
              className={
                item.id === providerId ? styles.providerOptionActive : styles.providerOption
              }
              key={item.id}
              disabled={disabled}
              onClick={() => onChange(item.id)}
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
  );
}

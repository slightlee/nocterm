import type { AiModelOption } from '../model/ai-model-catalog';
import { REASONING_LABELS, type AiReasoningEffort } from '../model/ai-reasoning';
import styles from './AiModelPicker.module.css';

interface Props {
  model?: AiModelOption;
  selected?: AiReasoningEffort;
  disabled: boolean;
  onBack: () => void;
  onSelect: (effort?: AiReasoningEffort) => void;
}

/** 右栏只展示左侧模型的能力；点击档位才一次提交模型和强度，悬停不会改变会话。 */
export function AiReasoningOptions({ model, selected, disabled, onBack, onSelect }: Props) {
  const options = model?.reasoningEfforts ?? [];
  return (
    <div className={styles.content}>
      <div className={styles.reasoningHeader}>
        <div>
          <strong>思考强度</strong>
          <span title={model?.name}>{model?.name ?? '选择模型'}</span>
        </div>
      </div>
      <div className={styles.scrollArea}>
        <div
          className={styles.modelList}
          role="listbox"
          aria-label="思考强度"
          onKeyDown={(event) => {
            // 左右键在两栏间移动，上下键仅在本组有效档位间移动。
            if (event.nativeEvent.isComposing) return;
            if (event.key === 'ArrowLeft') {
              event.preventDefault();
              onBack();
              return;
            }
            if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
            event.preventDefault();
            const buttons = Array.from(
              event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')
            );
            const index = buttons.indexOf(event.target as HTMLButtonElement);
            buttons[index + (event.key === 'ArrowDown' ? 1 : -1)]?.focus();
          }}
        >
          {options.map((effort) => (
            <button
              key={effort}
              className={styles.option}
              type="button"
              role="option"
              disabled={disabled}
              aria-selected={selected === effort}
              onClick={() => onSelect(effort)}
            >
              <span>{REASONING_LABELS[effort]}</span>
              {model?.defaultReasoningEffort === effort && (
                <small className={styles.defaultEffort}>默认</small>
              )}
              {selected === effort && (
                <svg className={styles.check} viewBox="0 0 24 24" aria-hidden="true">
                  <path d="m5 12 4 4 10-10" />
                </svg>
              )}
            </button>
          ))}
        </div>
        {model && options.length > 0 && !model.defaultReasoningEffort && !selected && !disabled && (
          <small className={styles.notice}>未选择档位时，由 Agent 决定。</small>
        )}
        {!model?.reasoningEfforts?.length && (
          <small className={styles.notice}>
            {model ? '此模型未提供思考强度选项。' : '选择左侧模型，查看可用的思考强度。'}
          </small>
        )}
      </div>
    </div>
  );
}

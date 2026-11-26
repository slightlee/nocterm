import { useEffect, useRef, useState } from 'react';

import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import { refreshModelCatalog, type CatalogSnapshot } from '../api/ai-model-catalog';
import { useAiAgentPreferences } from '../hooks/use-ai-agent-preferences';
import { useAiModelCatalog } from '../hooks/use-ai-model-catalog';
import { applyModelPreferences } from '../model/ai-agent-preferences';
import { filterModelOptions, modelUnavailableLabel } from '../model/ai-model-catalog';
import type { AiProvider } from '../model/ai-types';
import { compatibleReasoningEffort, type AiReasoningEffort } from '../model/ai-reasoning';
import { AiProviderIcon } from './AiProviderIcon';
import { AiReasoningOptions } from './AiReasoningOptions';
import styles from './AiModelPicker.module.css';

interface Props {
  provider: AiProvider;
  selected?: string;
  selectedEffort?: AiReasoningEffort;
  active: boolean;
  disabled: boolean;
  onSelect: (model: string, effort?: AiReasoningEffort) => void;
}

/** 刷新旧目录时继续显示上次结果，仅在没有任何已知结果时显示加载提示。 */
function modelCatalogStatus({ models, error }: CatalogSnapshot): string {
  if (!isDesktopRuntime()) return '本机模型在桌面客户端中显示';
  if (error === 'not-installed') return '此 Agent 尚未安装';
  if (error)
    return models?.length
      ? '刷新失败，显示上次读取的模型'
      : '模型列表读取失败，可在设置 → 智能体中添加自定义模型';
  if (!models) return '正在读取本机模型…';
  return models.length ? '' : '当前 Agent 未报告模型';
}

/** 按 Provider 挂载；切换或关闭菜单以后，迟到的 CLI 响应不能覆盖新列表。 */
export function AiModelOptions({
  provider: agent,
  selected,
  selectedEffort,
  active,
  disabled,
  onSelect,
}: Props) {
  const provider = agent.id;
  const [query, setQuery] = useState('');
  const [showLegacy, setShowLegacy] = useState(false);
  const [reasoningModelId, setReasoningModelId] = useState(active ? selected : undefined);
  const listRef = useRef<HTMLDivElement>(null);
  const reasoningRef = useRef<HTMLDivElement>(null);
  const catalog = useAiModelCatalog(provider);
  const preferences = useAiAgentPreferences().providers[provider];
  const status = modelCatalogStatus(catalog);
  useEffect(() => {
    if (!isDesktopRuntime()) return;
    void refreshModelCatalog(provider);
  }, [provider]);

  const options = applyModelPreferences(
    filterModelOptions(
      catalog.models ?? [],
      preferences.customModels,
      active ? selected : undefined,
      query,
      showLegacy
    ),
    preferences
  );
  const legacyCount =
    catalog.models?.filter(
      (model) =>
        model.source === 'catalog' &&
        model.isLegacy &&
        !preferences.disabledModels.includes(model.id)
    ).length ?? 0;
  // 预览不提交选择；从可见列表读取最新能力，搜索隐藏模型后不能误选隐藏项的档位。
  const reasoningModel = options.find((model) => model?.id === reasoningModelId);
  const focusReasoning = () => {
    reasoningRef.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
  };
  const focusModel = () => {
    const rows = listRef.current?.querySelectorAll<HTMLButtonElement>('[role="option"]');
    Array.from(rows ?? [])
      .find((row) => row.dataset.modelId === (reasoningModel?.id ?? ''))
      ?.focus();
  };
  // 只报告选模意图；保留已有档位或采用默认值由控制器统一决定。
  const selectModel = (id: string) => onSelect(id);
  return (
    <div className={styles.columns}>
      <div className={styles.content}>
        <label className={styles.search}>
          <svg className={styles.lineIcon} viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="10.5" cy="10.5" r="6.5" />
            <path d="m16 16 4.5 4.5" />
          </svg>
          <input
            aria-label={`搜索 ${agent.name} 模型`}
            placeholder={`搜索 ${agent.name} 模型…`}
            autoFocus
            autoComplete="off"
            spellCheck={false}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              // 搜索框在聊天表单内；Enter 只能选择搜索结果，不能提交终端请求。
              if (event.key === 'Enter') {
                event.preventDefault();
                const first = options.find((model) => !model?.unavailableReason);
                if (!event.nativeEvent.isComposing && !disabled && query.trim() && first)
                  selectModel(first.id);
              } else if (event.key === 'ArrowDown' && !event.nativeEvent.isComposing) {
                event.preventDefault();
                listRef.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
              }
            }}
          />
        </label>
        <div className={styles.scrollArea}>
          <div
            className={styles.modelList}
            role="listbox"
            aria-label="可选模型"
            ref={listRef}
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing) return;
              if (event.key === 'ArrowRight') {
                event.preventDefault();
                focusReasoning();
                return;
              }
              if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
              event.preventDefault();
              const buttons = Array.from(
                event.currentTarget.querySelectorAll<HTMLButtonElement>(
                  '[role="option"]:not(:disabled)'
                )
              );
              const index = buttons.indexOf(event.target as HTMLButtonElement);
              const next = index + (event.key === 'ArrowDown' ? 1 : -1);
              buttons[next]?.focus();
            }}
          >
            {/* 来源只参与目录处理；所有模型在同一列表中按原始调用 ID 选择。 */}
            {options.map((model) => (
              <div
                key={model.id}
                className={styles.optionRow}
                role="presentation"
                data-preview={reasoningModel?.id === model.id}
                onMouseEnter={() => setReasoningModelId(model.id)}
                onFocus={() => setReasoningModelId(model.id)}
              >
                <button
                  type="button"
                  role="option"
                  data-model-id={model.id}
                  aria-selected={active && selected === model.id}
                  disabled={disabled || !!model.unavailableReason}
                  className={styles.option}
                  title={[model.name, model.id, model.resolvedModel, modelUnavailableLabel(model)]
                    .filter(Boolean)
                    .join(' · ')}
                  onClick={() => selectModel(model.id)}
                >
                  <AiProviderIcon className={styles.rowBrand} providerId={provider} />
                  <span className={styles.modelText}>
                    <span>{model.name}</span>
                    {/* 常规模型不重复展示 ID；映射别名保留行内标识，避免同名项无法区分。 */}
                    {model.name !== model.id &&
                      (model.resolvedModel ||
                        options.some(
                          (item) => item.id !== model.id && item.name === model.name
                        )) && <small className={styles.alias}>{model.id}</small>}
                  </span>
                  {model.unavailableReason && (
                    <small className={styles.availability}>
                      {model.unavailableReason === 'cliUpdateRequired' ? '需更新' : '版本未知'}
                    </small>
                  )}
                  {active && selected === model.id && (
                    <svg className={styles.check} viewBox="0 0 24 24" aria-hidden="true">
                      <path d="m5 12 4 4 10-10" />
                    </svg>
                  )}
                </button>
              </div>
            ))}
          </div>
          {!query.trim() && legacyCount > 0 && (
            <button
              type="button"
              className={styles.add}
              aria-expanded={showLegacy}
              disabled={disabled}
              onClick={() => setShowLegacy((value) => !value)}
            >
              {showLegacy ? '收起历史版本' : `显示历史版本（${legacyCount}）`}
            </button>
          )}
          {(status || !options.length) && (
            <small role="status" className={styles.notice}>
              {status || '没有匹配的模型'}
            </small>
          )}
        </div>
      </div>
      <div className={styles.reasoningPane} ref={reasoningRef}>
        <AiReasoningOptions
          model={reasoningModel}
          disabled={disabled || !!reasoningModel?.unavailableReason}
          selected={compatibleReasoningEffort(
            reasoningModel?.reasoningEfforts,
            active && selected === reasoningModel?.id ? selectedEffort : undefined
          )}
          onBack={focusModel}
          onSelect={(effort) => {
            if (reasoningModel) onSelect(reasoningModel.id, effort);
          }}
        />
      </div>
    </div>
  );
}

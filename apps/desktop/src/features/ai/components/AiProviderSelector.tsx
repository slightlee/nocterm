import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import { useAiAgentPreferences } from '../hooks/use-ai-agent-preferences';
import { useAiModelCatalog } from '../hooks/use-ai-model-catalog';
import { enabledProviders } from '../model/ai-agent-preferences';
import { REASONING_LABELS, type AiReasoningEffort } from '../model/ai-reasoning';
import type { AiProvider, AiProviderId } from '../model/ai-types';
import { modelMenuPosition } from './ai-model-menu-position';
import { AiModelOptions } from './AiModelOptions';
import { AiProviderIcon } from './AiProviderIcon';
import styles from './AiComposerControls.module.css';
import picker from './AiModelPicker.module.css';

interface AiProviderSelectorProps {
  provider: AiProvider;
  providerId: AiProviderId;
  model?: string;
  reasoningEffort?: AiReasoningEffort;
  fastMode?: boolean;
  supportsFastMode?: boolean;
  open: boolean;
  disabled: boolean;
  onToggle: () => void;
  onChange: (
    provider: AiProviderId,
    model?: string,
    effort?: AiReasoningEffort,
    fastMode?: boolean
  ) => void;
}

/** Provider 选择器只呈现已注册 Adapter，不参与会话启动或可用性探测。 */
export function AiProviderSelector({
  provider,
  providerId,
  model,
  reasoningEffort,
  fastMode,
  supportsFastMode,
  open,
  disabled,
  onToggle,
  onChange,
}: AiProviderSelectorProps) {
  // 标签只影响展示，选择与发送始终使用原始 ID，避免把映射名称当作调用参数。
  const selectedModel = useAiModelCatalog(providerId).models?.find((item) => item.id === model);
  const modelName = selectedModel?.name;
  const menuId = useId();
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  // 弹层挂到 body，脱离窄 AI 面板的 container/overflow；随锚点和窗口变化重新定位。
  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const anchor = trigger.current?.getBoundingClientRect();
      const element = menu.current;
      if (!anchor || !element) return;
      Object.assign(
        element.style,
        modelMenuPosition(anchor, window.innerWidth, window.innerHeight)
      );
    };
    place();
    const observer = new ResizeObserver(place);
    if (trigger.current) observer.observe(trigger.current);
    // 面板拖宽时锚点可能只移动、不改变尺寸，因此也观察其面板容器。
    const panel = root.current?.closest('aside');
    if (panel) observer.observe(panel);
    window.addEventListener('resize', place);
    document.addEventListener('scroll', place, true);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', place);
      document.removeEventListener('scroll', place, true);
    };
  }, [open, model]);
  // 工具栏位于表单内部。点击菜单以外的控件时，让原点击继续执行。
  useEffect(() => {
    if (!open) return;
    const dismissOutside = (event: Event) => {
      if (
        event.target instanceof Node &&
        !root.current?.contains(event.target) &&
        !menu.current?.contains(event.target)
      )
        onToggle();
    };
    document.addEventListener('pointerdown', dismissOutside);
    document.addEventListener('focusin', dismissOutside);
    return () => {
      document.removeEventListener('pointerdown', dismissOutside);
      document.removeEventListener('focusin', dismissOutside);
    };
  }, [open, onToggle]);
  return (
    <div className={picker.anchor} ref={root}>
      <button
        ref={trigger}
        aria-controls={open ? menuId : undefined}
        aria-haspopup="dialog"
        aria-expanded={open}
        className={`${styles.providerButton} ${styles.composerProviderButton}`}
        disabled={disabled}
        onClick={onToggle}
        title={model ? provider.name + ' · ' + model : '选择 Agent 和模型'}
        type="button"
      >
        <span className={styles.providerLabel}>
          {/* 未选模型时不展示沿用的 Agent 品牌，避免把默认浏览位置误认为已选项。 */}
          {model ? (
            <AiProviderIcon className={styles.agentProviderIcon} providerId={providerId} />
          ) : (
            <svg className={styles.modelPlaceholderIcon} viewBox="0 0 24 24" aria-hidden="true">
              <path d="m12 3 9 5-9 5-9-5 9-5ZM3 12l9 5 9-5M3 16l9 5 9-5" />
            </svg>
          )}
          <span>{modelName ?? model ?? '选择模型'}</span>
          {reasoningEffort && (
            <small className={picker.effortBadge}>{REASONING_LABELS[reasoningEffort]}</small>
          )}
          {fastMode && supportsFastMode && (
            <svg className={picker.speedIcon} viewBox="0 0 24 24" role="img" aria-label="快速模式">
              <path d="m13 2-10 12h9l-1 8 10-12h-9l1-8Z" />
            </svg>
          )}
        </span>
        <svg className={styles.providerChevron} viewBox="0 0 24 24" aria-hidden="true">
          <path d="m7 10 5 5 5-5" />
        </svg>
      </button>
      {open
        ? createPortal(
            <div
              ref={menu}
              id={menuId}
              className={picker.menu}
              role="dialog"
              aria-label="Agent 和模型"
              tabIndex={-1}
              onKeyDown={(event) => {
                if (event.key === 'Escape') {
                  if (event.nativeEvent.isComposing) return;
                  event.preventDefault();
                  event.stopPropagation();
                  trigger.current?.focus();
                  onToggle();
                }
              }}
            >
              <ModelMenuContent
                providerId={providerId}
                model={model}
                reasoningEffort={reasoningEffort}
                fastMode={fastMode}
                supportsFastMode={supportsFastMode}
                disabled={disabled}
                onChange={(agent, id, effort, speed) => {
                  onChange(agent, id, effort, speed);
                }}
              />
            </div>,
            document.body
          )
        : null}
    </div>
  );
}

/** 每次打开重新初始化浏览状态；已有选择优先，否则使用当前有序列表的第一项。 */
function ModelMenuContent({
  providerId,
  model,
  reasoningEffort,
  fastMode,
  supportsFastMode,
  disabled,
  onChange,
}: Pick<
  AiProviderSelectorProps,
  | 'providerId'
  | 'model'
  | 'reasoningEffort'
  | 'fastMode'
  | 'supportsFastMode'
  | 'disabled'
  | 'onChange'
>) {
  const providers = enabledProviders(useAiAgentPreferences());
  const [browsing, setBrowsing] = useState<AiProviderId>(
    model && providers.some(({ id }) => id === providerId)
      ? providerId
      : (providers[0]?.id ?? providerId)
  );
  const agent = providers.find((item) => item.id === browsing) ?? providers[0];
  const speedAvailable = agent?.id === providerId && supportsFastMode === true;
  const emptyState = useRef<HTMLDivElement>(null);
  useEffect(() => {
    // 全部停用时没有搜索框接收焦点，仍需让 Esc 可关闭弹层并返回工具栏。
    if (!agent) emptyState.current?.focus();
  }, [agent]);
  return (
    <>
      <div className={picker.rail} role="group" aria-label="Agent">
        {providers.map((item) => (
          <button
            aria-pressed={item.id === agent?.id}
            aria-label={item.name}
            title={item.name}
            className={picker.agent}
            key={item.id}
            disabled={disabled}
            onClick={() => setBrowsing(item.id)}
            type="button"
          >
            <AiProviderIcon className={picker.brand} providerId={item.id} />
          </button>
        ))}
      </div>
      {!agent && (
        <div className={picker.emptyProviders} tabIndex={-1} ref={emptyState}>
          <strong>没有启用的 Agent</strong>
          <span>请在“设置 → 智能体”中启用至少一个 Agent。</span>
        </div>
      )}
      {agent && (
        <AiModelOptions
          key={agent.id}
          provider={agent}
          active={agent.id === providerId}
          selected={model}
          selectedEffort={reasoningEffort}
          disabled={disabled}
          onSelect={(id, effort) => onChange(agent.id, id, effort)}
        />
      )}
      {speedAvailable && model && (
        <div className={picker.speedFooter}>
          <div className={picker.speedOptions} role="group" aria-label="响应速度">
            <button
              type="button"
              disabled={disabled}
              aria-pressed={fastMode !== true}
              onClick={() => onChange(providerId, model, reasoningEffort, false)}
            >
              标准
            </button>
            <button
              type="button"
              disabled={disabled}
              aria-pressed={fastMode === true}
              onClick={() => onChange(providerId, model, reasoningEffort, true)}
              title="同一模型更低延迟，可能消耗更多额度"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="m13 2-10 12h9l-1 8 10-12h-9l1-8Z" />
              </svg>
              快速
            </button>
          </div>
        </div>
      )}
    </>
  );
}

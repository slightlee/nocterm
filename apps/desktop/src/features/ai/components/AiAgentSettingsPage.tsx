import { useEffect, useMemo, useRef, useState } from 'react';

import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import {
  addAiCustomModel,
  moveAiModel,
  moveAiProvider,
  removeAiCustomModel,
  setAiModelEnabled,
  setAiProviderEnabled,
} from '../api/ai-agent-preferences';
import { refreshModelCatalog, refreshProviderVersion } from '../api/ai-model-catalog';
import { useAiAgentPreferences } from '../hooks/use-ai-agent-preferences';
import { useAiModelCatalog } from '../hooks/use-ai-model-catalog';
import { applyModelPreferences } from '../model/ai-agent-preferences';
import { filterModelOptions, isModelId, modelUnavailableLabel } from '../model/ai-model-catalog';
import { providerStatus } from '../model/ai-provider-status';
import { AI_PROVIDERS, type AiProvider, type AiProviderId } from '../model/ai-types';
import { AiProviderIcon } from './AiProviderIcon';
import styles from './AiAgentSettingsPage.module.css';

function AgentIdentity({ provider }: { provider: AiProvider }) {
  const snapshot = useAiModelCatalog(provider.id);
  const status = providerStatus(snapshot, isDesktopRuntime());

  useEffect(() => {
    if (!isDesktopRuntime()) return;
    void refreshModelCatalog(provider.id);
    void refreshProviderVersion(provider.id);
  }, [provider.id]);

  return (
    <span className={styles.agentCopy}>
      <span className={styles.agentName}>
        <strong>{provider.name}</strong>
        <span className={styles.version} title="本机 CLI 版本">
          {snapshot.cliVersion
            ? `v${snapshot.cliVersion}`
            : snapshot.cliVersion === null
              ? '版本未知'
              : '—'}
        </span>
      </span>
      {status && (
        <small className={styles[status.tone]} role="status">
          {status.label}
        </small>
      )}
    </span>
  );
}

/** 设置页管理 Nocterm 中的可见性与顺序，不接管各 CLI 的账号、网关或环境变量。 */
export function AiAgentSettingsPage() {
  const preferences = useAiAgentPreferences();
  const providers = preferences.providerOrder
    .map((id) => AI_PROVIDERS.find((provider) => provider.id === id))
    .filter((provider): provider is AiProvider => provider !== undefined);
  const [selected, setSelected] = useState<AiProviderId>(providers[0]?.id ?? 'codex');
  const [feedback, setFeedback] = useState('');
  const activeProvider = providers.find(({ id }) => id === selected) ?? providers[0];

  const change = (action: () => void): boolean => {
    try {
      action();
      setFeedback('');
      return true;
    } catch {
      setFeedback('无法保存智能体设置，请检查本地存储。');
      return false;
    }
  };

  return (
    <section className={styles.page}>
      <header className={styles.pageHeader}>
        <div>
          <h2>智能体</h2>
          <p>选择要显示的 Agent 和模型，按使用习惯排列。</p>
        </div>
      </header>

      <div className={styles.workspace}>
        <aside className={styles.agentPane} aria-label="Agent 列表">
          <div className={styles.paneHeader}>
            <strong>Agent</strong>
            <span>
              {providers.filter(({ id }) => preferences.providers[id].enabled).length} 个启用
            </span>
          </div>
          <div className={styles.agentList}>
            {providers.map((provider, index) => {
              const settings = preferences.providers[provider.id];
              return (
                <div
                  className={`${styles.agentRow} ${selected === provider.id ? styles.agentSelected : ''}`}
                  key={provider.id}
                >
                  <button
                    className={styles.agentSelect}
                    type="button"
                    aria-pressed={selected === provider.id}
                    onClick={() => setSelected(provider.id)}
                  >
                    <AiProviderIcon className={styles.agentIcon} providerId={provider.id} />
                    <AgentIdentity provider={provider} />
                  </button>
                  <div className={styles.rowActions}>
                    <OrderButtons
                      label={provider.name}
                      first={index === 0}
                      last={index === providers.length - 1}
                      onMove={(direction) => change(() => moveAiProvider(provider.id, direction))}
                    />
                    <Switch
                      label={`${settings.enabled ? '停用' : '启用'} ${provider.name}`}
                      checked={settings.enabled}
                      onChange={(enabled) =>
                        change(() => setAiProviderEnabled(provider.id, enabled))
                      }
                    />
                  </div>
                </div>
              );
            })}
          </div>
        </aside>

        {activeProvider && (
          <ModelSettings key={activeProvider.id} provider={activeProvider} change={change} />
        )}
      </div>
      {feedback && (
        <p className={styles.feedback} role="alert">
          {feedback}
        </p>
      )}
    </section>
  );
}

function ModelSettings({
  provider,
  change,
}: {
  provider: AiProvider;
  change: (action: () => void) => boolean;
}) {
  const preferences = useAiAgentPreferences().providers[provider.id];
  const catalog = useAiModelCatalog(provider.id);
  const [query, setQuery] = useState('');
  const [draft, setDraft] = useState('');
  const [error, setError] = useState('');
  const [refreshing, setRefreshing] = useState(false);
  const [adding, setAdding] = useState(false);
  const addTrigger = useRef<HTMLButtonElement>(null);
  const allModels = useMemo(
    () =>
      applyModelPreferences(
        filterModelOptions(catalog.models ?? [], preferences.customModels, undefined, '', true),
        preferences,
        true
      ),
    [catalog.models, preferences]
  );
  const normalizedQuery = query.trim().toLowerCase();
  const visibleModels = normalizedQuery
    ? allModels.filter((model) =>
        [model.name, model.id, model.resolvedModel ?? '']
          .join(' ')
          .toLowerCase()
          .includes(normalizedQuery)
      )
    : allModels;

  // Agent 列表负责自动探测；手动刷新复用同一请求，切换页面不会取消其他订阅。
  const refresh = async () => {
    setRefreshing(true);
    await Promise.all([
      refreshModelCatalog(provider.id, { force: true }),
      refreshProviderVersion(provider.id, true),
    ]);
    setRefreshing(false);
  };
  const addModel = () => {
    const id = draft.trim();
    if (!isModelId(id)) {
      setError('请输入有效的模型 ID。');
      return;
    }
    const unavailable = modelUnavailableLabel(catalog.models?.find((model) => model.id === id));
    if (unavailable) {
      setError(unavailable);
      return;
    }
    if (allModels.some((model) => model.id === id)) {
      setError('该模型已经在列表中。');
      return;
    }
    if (change(() => addAiCustomModel(provider.id, id))) {
      setDraft('');
      setError('');
      setAdding(false);
      addTrigger.current?.focus();
    }
  };

  return (
    <section className={styles.modelPane} aria-labelledby={`agent-${provider.id}`}>
      <header className={styles.modelHeader}>
        <h3 id={`agent-${provider.id}`}>模型</h3>
        <div className={styles.modelMeta}>
          <button
            className={styles.refreshButton}
            type="button"
            aria-label="重新检测模型与 CLI 版本"
            disabled={!isDesktopRuntime() || refreshing}
            onClick={() => void refresh()}
            title={refreshing ? '正在检测' : '重新检测模型与 CLI 版本'}
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M20 7v5h-5M4 17v-5h5" />
              <path d="M6.1 8.2A7 7 0 0 1 18.5 7M17.9 15.8A7 7 0 0 1 5.5 17" />
            </svg>
          </button>
          {catalog.refreshedAt && (
            <span className={styles.muted}>
              最近检测 {new Date(catalog.refreshedAt).toLocaleTimeString()}
            </span>
          )}
        </div>
      </header>

      {catalog.error && (
        <p className={styles.hint} role="status">
          {catalog.error === 'unavailable'
            ? '读取失败，保留上次目录。请检查本机 CLI 后重试。'
            : '尚未安装此 Agent，安装后可刷新模型。'}
        </p>
      )}

      <div className={styles.toolbar}>
        <label className={styles.search}>
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="10.5" cy="10.5" r="6.5" />
            <path d="m16 16 4.5 4.5" />
          </svg>
          <input
            aria-label={`搜索 ${provider.name} 模型`}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索名称或模型 ID"
          />
        </label>
        <div className={styles.toolbarActions}>
          <span>{allModels.length} 个模型</span>
          <button
            ref={addTrigger}
            type="button"
            className={styles.addModelButton}
            aria-expanded={adding}
            aria-controls={`custom-${provider.id}`}
            onClick={() => setAdding(!adding)}
          >
            + 添加自定义模型
          </button>
        </div>
      </div>

      {adding && (
        <form
          id={`custom-${provider.id}`}
          className={styles.customForm}
          onKeyDown={(event) => {
            if (event.key === 'Escape' && !event.nativeEvent.isComposing) {
              event.preventDefault();
              setAdding(false);
              addTrigger.current?.focus();
            }
          }}
          onSubmit={(event) => {
            event.preventDefault();
            addModel();
          }}
        >
          <label>
            <span className={styles.visuallyHidden}>模型 ID</span>
            <input
              autoFocus
              value={draft}
              maxLength={200}
              spellCheck={false}
              autoComplete="off"
              onChange={(event) => {
                setDraft(event.target.value);
                setError('');
              }}
              placeholder="例如 vendor/model-id"
            />
          </label>
          <button className={styles.addButton} type="submit" disabled={!draft.trim()}>
            添加
          </button>
          <button
            type="button"
            className={styles.addModelButton}
            onClick={() => {
              setAdding(false);
              addTrigger.current?.focus();
            }}
          >
            取消
          </button>
          {error && <small role="alert">{error}</small>}
        </form>
      )}

      <div className={styles.modelList}>
        {visibleModels.map((model) => {
          const index = allModels.findIndex(({ id }) => id === model.id);
          const enabled = !preferences.disabledModels.includes(model.id);
          // 同 ID 后来被 CLI 收录时仍允许删除本地自定义记录，目录项继续保留。
          const custom = preferences.customModels.includes(model.id);
          return (
            <div className={styles.modelRow} key={model.id}>
              <div
                className={styles.modelIdentity}
                title={[model.name, model.id, modelUnavailableLabel(model)]
                  .filter(Boolean)
                  .join(' · ')}
              >
                <strong>{model.name}</strong>
                {model.name !== model.id && <span>{model.id}</span>}
                {custom && <small>手动添加</small>}
                {model.unavailableReason && <small>{modelUnavailableLabel(model)}</small>}
              </div>
              <div className={styles.rowActions}>
                <OrderButtons
                  label={model.name}
                  first={index === 0}
                  last={index === allModels.length - 1}
                  onMove={(direction) =>
                    change(() =>
                      moveAiModel(
                        provider.id,
                        allModels.map(({ id }) => id),
                        model.id,
                        direction
                      )
                    )
                  }
                />
                {custom && (
                  <button
                    className={styles.iconButton}
                    type="button"
                    aria-label={`删除自定义模型 ${model.id}`}
                    title="删除自定义模型"
                    onClick={() => change(() => removeAiCustomModel(provider.id, model.id))}
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path d="M4 7h16M9 7V4h6v3M8 10v8M12 10v8M16 10v8M6 7l1 14h10l1-14" />
                    </svg>
                  </button>
                )}
                <Switch
                  label={`${enabled ? '停用' : '启用'}模型 ${model.id}`}
                  checked={enabled}
                  onChange={(checked) =>
                    change(() => setAiModelEnabled(provider.id, model.id, checked))
                  }
                />
              </div>
            </div>
          );
        })}
        {!visibleModels.length && (
          <div className={styles.emptyState}>
            <strong>{query.trim() ? '没有匹配的模型' : '还没有模型'}</strong>
            <span>
              {query.trim() ? '尝试其他名称或模型 ID。' : '刷新模型，或添加自定义模型 ID。'}
            </span>
          </div>
        )}
      </div>
    </section>
  );
}

function OrderButtons({
  label,
  first,
  last,
  onMove,
}: {
  label: string;
  first: boolean;
  last: boolean;
  onMove: (direction: -1 | 1) => void;
}) {
  return (
    <span className={styles.orderButtons}>
      <button
        className={styles.orderButton}
        type="button"
        disabled={first}
        aria-label={`上移 ${label}`}
        title={`上移 ${label}`}
        onClick={() => onMove(-1)}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M12 19V5M6.5 10.5 12 5l5.5 5.5" />
        </svg>
      </button>
      <button
        className={styles.orderButton}
        type="button"
        disabled={last}
        aria-label={`下移 ${label}`}
        title={`下移 ${label}`}
        onClick={() => onMove(1)}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M12 5v14M6.5 13.5 12 19l5.5-5.5" />
        </svg>
      </button>
    </span>
  );
}

function Switch({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <button
      className={`${styles.switch} ${checked ? styles.switchOn : ''}`}
      type="button"
      role="switch"
      aria-label={label}
      aria-checked={checked}
      onClick={() => onChange(!checked)}
    />
  );
}

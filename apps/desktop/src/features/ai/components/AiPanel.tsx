import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type FormEvent,
  type KeyboardEvent,
} from 'react';

import { useTerminalStore, type TerminalSessionId } from '../../terminal';
import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import { resetAiConversation } from '../api/ai-client';
import { useAiPersistence } from '../hooks/use-ai-persistence';
import { useAiRuntime } from '../hooks/use-ai-runtime';
import { readAiAttachment, type AiAttachment } from '../model/ai-attachment';
import {
  buildInputHistory,
  navigateInputHistory,
  type AiHistoryBrowseState,
} from '../model/ai-input-history';
import {
  conversationTargetKey,
  resolveActiveAiConversation,
  resolveAiComposerState,
  useAiStore,
} from '../model/ai-store';
import { canBeginAiConversation, resolveAiTaskNavigation } from '../model/ai-panel-state';
import {
  findAiRuntimeTaskByConversation,
  findAiRuntimeTaskByTarget,
  listBackgroundAiRuntimeTasks,
  listCompletedAiTaskResults,
  type AiTaskNavigationSource,
} from '../model/ai-runtime-task';
import {
  aiConversationTargetKey,
  aiConversationTargetsEqual,
  isAiTerminalTargetReady,
  resolveAiWorkspaceTarget,
  type AiWorkspaceView,
} from '../model/ai-target';
import { AI_PROVIDERS, type AiCommandPolicy, type AiProviderId } from '../model/ai-types';
import { AiComposer } from './AiComposer';
import { AiConversationView } from './AiConversationView';
import { AiHistoryMenu } from './AiHistoryMenu';
import { AiPanelToggle } from './AiPanelToggle';
import { AiTaskMenu } from './AiTaskMenu';
import styles from './AiPanel.module.css';

interface AiPanelProps {
  activeView: AiWorkspaceView;
  onActivateTerminal: (sessionId: TerminalSessionId) => void;
  visible: boolean;
}

/** AI 工作台常驻接收任务事件；Tab 切换只选择目标视图，不创建空会话。 */
export function AiPanel({ activeView, onActivateTerminal, visible }: AiPanelProps) {
  const sessions = useTerminalStore((state) => state.sessions);
  const activeId = useTerminalStore((state) => state.activeId);
  const recentRemoteSessionIds = useTerminalStore((state) => state.recentRemoteSessionIds);
  const terminalStatuses = useTerminalStore((state) => state.statuses);
  const conversations = useAiStore((state) => state.conversations);
  const activeConversationIds = useAiStore((state) => state.activeConversationIds);
  const composers = useAiStore((state) => state.composers);
  const setOpen = useAiStore((state) => state.setOpen);
  const setProvider = useAiStore((state) => state.setProvider);
  const setCommandPolicy = useAiStore((state) => state.setCommandPolicy);
  const setDraft = useAiStore((state) => state.setDraft);
  const beginConversation = useAiStore((state) => state.beginConversation);
  const addMessage = useAiStore((state) => state.addMessage);
  const deleteMessage = useAiStore((state) => state.deleteMessage);
  const clearMessages = useAiStore((state) => state.clearMessages);
  const createConversation = useAiStore((state) => state.createConversation);
  const deleteConversation = useAiStore((state) => state.deleteConversation);
  const selectConversation = useAiStore((state) => state.selectConversation);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [taskMenuOpen, setTaskMenuOpen] = useState(false);
  const [providerMenuOpen, setProviderMenuOpen] = useState(false);
  const [inspectedConversationId, setInspectedConversationId] = useState<string | null>(null);
  const [attachments, setAttachments] = useState<Record<string, AiAttachment | null>>({});
  const [attachmentLoadingTargetKey, setAttachmentLoadingTargetKey] = useState<string | null>(null);
  const [panelNotices, setPanelNotices] = useState<Record<string, string | null>>({});
  const [persistenceNotice, setPersistenceNotice] = useState<string | null>(null);
  const [taskNavigationNotice, setTaskNavigationNotice] = useState<{
    conversationId: string;
    message: string;
  } | null>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  // 历史输入浏览状态属于当前目标；Tab 变化时必须清空，不能把 A 的游标带到 B。
  const historyBrowserRef = useRef<AiHistoryBrowseState | null>(null);

  const activeSession = useMemo(
    () => sessions.find((session) => session.id === activeId) ?? null,
    [activeId, sessions]
  );
  const activeSessionStatus = activeSession
    ? (terminalStatuses[String(activeSession.id)] ?? 'connecting')
    : 'idle';
  const workspaceTarget = useMemo(
    () => resolveAiWorkspaceTarget(activeView, activeSession, activeSessionStatus),
    [activeSession, activeSessionStatus, activeView]
  );
  const workspaceTargetKey = workspaceTarget.binding
    ? aiConversationTargetKey(workspaceTarget.binding)
    : null;
  const panelViewKey = workspaceTargetKey ?? `view:${activeView}`;
  const targetConversation = useMemo(
    () =>
      workspaceTargetKey
        ? resolveActiveAiConversation(conversations, activeConversationIds, workspaceTargetKey)
        : null,
    [activeConversationIds, conversations, workspaceTargetKey]
  );
  const inspectedConversation = inspectedConversationId
    ? (conversations.find((conversation) => conversation.id === inspectedConversationId) ?? null)
    : null;
  const activeConversation = inspectedConversation ?? targetConversation;
  const activeConversationId = activeConversation?.id ?? null;
  const viewingCurrentTarget = activeConversation
    ? aiConversationTargetsEqual(activeConversation.target, workspaceTarget.binding)
    : Boolean(workspaceTarget.binding);
  const composerState = workspaceTargetKey
    ? resolveAiComposerState(composers, workspaceTargetKey)
    : { draft: '', provider: 'codex' as const, commandPolicy: 'auto_safe' as const };
  const provider = activeConversation?.provider ?? composerState.provider;
  const commandPolicy: AiCommandPolicy =
    activeConversation?.commandPolicy ?? composerState.commandPolicy;
  const currentTargetProvider = targetConversation?.provider ?? composerState.provider;
  const selectedProvider = AI_PROVIDERS.find((item) => item.id === provider) ?? AI_PROVIDERS[0];
  const messages = activeConversation?.messages ?? [];
  const draft = inspectedConversation ? '' : composerState.draft;
  const attachment =
    workspaceTargetKey && !inspectedConversation ? (attachments[workspaceTargetKey] ?? null) : null;
  const attachmentLoading = attachmentLoadingTargetKey === workspaceTargetKey;
  const targetReady = viewingCurrentTarget && isAiTerminalTargetReady(workspaceTarget);
  const {
    tasks,
    completedTaskResults,
    presentations,
    globalNotice,
    providerAvailability,
    sendQuestion,
    resolveApproval,
    requestSessionStop,
    markTaskResultRead,
    markAllTaskResultsRead,
    clearConversationPresentation,
  } = useAiRuntime({ addMessage });
  useAiPersistence({ onError: setPersistenceNotice });

  const activeTask = findAiRuntimeTaskByConversation(tasks, activeConversationId);
  const currentTargetTask = findAiRuntimeTaskByTarget(tasks, workspaceTargetKey);
  const backgroundTasks = listBackgroundAiRuntimeTasks(
    tasks,
    workspaceTargetKey,
    activeConversationId
  );
  const unreadTaskResults = listCompletedAiTaskResults(completedTaskResults).filter(
    (result) => !(visible && result.conversationId === activeConversationId)
  );
  const taskAttentionCount = backgroundTasks.length + unreadTaskResults.length;
  const taskAttentionBadge = taskAttentionCount > 99 ? '99+' : taskAttentionCount;
  const runningConversationIds = Object.values(tasks).map((task) => task.conversationId);
  const runningConversationIdSet = new Set(runningConversationIds);
  const activePresentation = activeConversationId ? presentations[activeConversationId] : undefined;
  const currentTargetBusyElsewhere = Boolean(currentTargetTask && !activeTask);
  const panelNotice = panelNotices[panelViewKey] ?? persistenceNotice;
  const visibleNotice =
    activeTask?.notice ??
    activePresentation?.notice ??
    (taskNavigationNotice?.conversationId === activeConversationId
      ? taskNavigationNotice.message
      : null) ??
    globalNotice ??
    panelNotice;
  const lastUserMessageId = activePresentation?.canRetryQuestion
    ? [...messages].reverse().find((message) => message.role === 'user')?.id
    : undefined;

  useEffect(() => {
    // 同一 SSH 连接的多个终端 Tab 共享 target key，因此不会误清其会话视图。
    // eslint-disable-next-line react-hooks/set-state-in-effect -- 目标身份切换必须原子退出跨目标历史查看态。
    setInspectedConversationId(null);
    setHistoryOpen(false);
    setTaskMenuOpen(false);
    setProviderMenuOpen(false);
    setTaskNavigationNotice(null);
    historyBrowserRef.current = null;
  }, [panelViewKey]);

  useEffect(() => {
    if (!visible || !activeConversationId || !completedTaskResults[activeConversationId]) return;
    markTaskResultRead(activeConversationId);
  }, [activeConversationId, completedTaskResults, markTaskResultRead, visible]);

  useEffect(() => {
    if (taskAttentionCount > 0) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- 入口消失时必须同步关闭浮层，避免后续任务自动展开。
    setTaskMenuOpen(false);
  }, [taskAttentionCount]);

  if (!visible) return null;

  const targetUnavailableMessage = (() => {
    if (activeConversation && !viewingCurrentTarget) {
      return activeConversation.target
        ? `该会话绑定 ${activeConversation.target.label}，请切回对应终端`
        : '旧版历史没有终端绑定，只可查看';
    }
    if (activeView === 'sftp') return '文件会话暂未接入 AI 执行上下文';
    if (!workspaceTarget.binding) return '请先连接本地终端或远程服务器';
    if (activeSessionStatus === 'connecting') return '终端连接完成后才能发送请求';
    if (activeSessionStatus === 'error') return '请先恢复终端连接';
    return '当前终端不可用';
  })();

  const clearCurrentAttachment = () => {
    if (!workspaceTargetKey) return;
    setAttachments((current) => ({ ...current, [workspaceTargetKey]: null }));
  };

  /** 异步操作必须把结果写回发起操作的目标，不能跟随之后切换到的 Tab。 */
  const setTargetPanelNotice = (targetKey: string, message: string | null) => {
    setPanelNotices((current) => ({ ...current, [targetKey]: message }));
  };

  const submit = (event?: FormEvent) => {
    event?.preventDefault();
    if (currentTargetTask || !workspaceTargetKey || !workspaceTarget.binding) return;
    const content = draft.trim();
    if (!content && !attachment) return;
    if (!targetReady) {
      setTargetPanelNotice(panelViewKey, `${targetUnavailableMessage}，暂时无法发送新请求。`);
      return;
    }

    const selectedAttachment = attachment;
    const question = content || `请分析附件：${selectedAttachment?.name ?? '未命名文件'}`;
    const visibleMessage = selectedAttachment
      ? `${question}\n\n附件：${selectedAttachment.name}`
      : question;
    const conversationId =
      activeConversation?.id ??
      createConversation(workspaceTarget.binding, provider, commandPolicy);
    const history = activeConversation?.messages ?? [];
    addMessage(conversationId, 'user', visibleMessage);
    setDraft(workspaceTargetKey, '');
    clearCurrentAttachment();
    setTargetPanelNotice(panelViewKey, null);
    sendQuestion({
      question,
      attachment: selectedAttachment,
      history,
      conversationId,
      provider,
      target: workspaceTarget,
      commandPolicy,
    });
  };

  /** 失败重试只能回到原目标和原会话；目标不匹配时禁止复制上下文执行。 */
  const retryLastQuestion = () => {
    if (currentTargetTask || !activeConversation || !targetReady) return;
    let lastUserIndex = -1;
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      if (messages[index]?.role === 'user') {
        lastUserIndex = index;
        break;
      }
    }
    const retried = lastUserIndex >= 0 ? messages[lastUserIndex] : undefined;
    if (!retried) return;
    const question = retried.content.replace(/\n\n附件：[^\n]*$/, '');
    messages
      .slice(lastUserIndex)
      .forEach((message) => deleteMessage(activeConversation.id, message.id));
    addMessage(activeConversation.id, 'user', question);
    sendQuestion({
      question,
      attachment: null,
      history: messages.slice(0, lastUserIndex),
      conversationId: activeConversation.id,
      provider,
      target: workspaceTarget,
      commandPolicy,
    });
  };

  const handleDraftChange = (value: string) => {
    if (!workspaceTargetKey) return;
    historyBrowserRef.current = null;
    setDraft(workspaceTargetKey, value);
  };

  const handleComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (!workspaceTargetKey || event.nativeEvent.isComposing) return;
    if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault();
      historyBrowserRef.current = null;
      submit();
      return;
    }
    if (event.key === 'Escape' && historyBrowserRef.current !== null) {
      event.preventDefault();
      setDraft(workspaceTargetKey, historyBrowserRef.current.savedDraft);
      historyBrowserRef.current = null;
      return;
    }
    if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;

    const latestState = useAiStore.getState();
    const latestComposer = resolveAiComposerState(latestState.composers, workspaceTargetKey);
    const entries = buildInputHistory(
      latestState.conversations.filter(
        (conversation) => conversationTargetKey(conversation) === workspaceTargetKey
      )
    );
    const browsing = historyBrowserRef.current !== null;
    if (!browsing) {
      if (event.key === 'ArrowDown') return;
      const { selectionStart } = event.currentTarget;
      if (latestComposer.draft.slice(0, selectionStart).includes('\n') || entries.length === 0)
        return;
      event.preventDefault();
      const step = navigateInputHistory(null, entries, latestComposer.draft, 'up');
      historyBrowserRef.current = step.browser;
      setDraft(workspaceTargetKey, step.text);
      return;
    }

    event.preventDefault();
    const step = navigateInputHistory(
      historyBrowserRef.current,
      entries,
      latestComposer.draft,
      event.key === 'ArrowUp' ? 'up' : 'down'
    );
    historyBrowserRef.current = step.browser;
    setDraft(workspaceTargetKey, step.text);
  };

  const handleAttachmentChange = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file || !workspaceTargetKey || !viewingCurrentTarget) return;

    const targetKey = workspaceTargetKey;
    setAttachmentLoadingTargetKey(targetKey);
    void readAiAttachment(file)
      .then((nextAttachment) => {
        setAttachments((current) => ({ ...current, [targetKey]: nextAttachment }));
        setTargetPanelNotice(targetKey, null);
      })
      .catch((error: unknown) => {
        setAttachments((current) => ({ ...current, [targetKey]: null }));
        setTargetPanelNotice(targetKey, error instanceof Error ? error.message : '读取附件失败。');
      })
      .finally(() =>
        setAttachmentLoadingTargetKey((current) => (current === targetKey ? null : current))
      );
  };

  const resetProviderConversation = async (id: string, noticeKey: string): Promise<boolean> => {
    if (!isDesktopRuntime()) return true;
    try {
      await resetAiConversation(id);
      return true;
    } catch (error: unknown) {
      setTargetPanelNotice(
        noticeKey,
        error instanceof Error ? error.message : '重置 AI 会话失败。'
      );
      return false;
    }
  };

  const handleProviderChange = async (nextProvider: AiProviderId) => {
    if (
      !workspaceTargetKey ||
      !viewingCurrentTarget ||
      currentTargetTask ||
      nextProvider === provider
    ) {
      setProviderMenuOpen(false);
      return;
    }
    if (
      activeConversation &&
      !(await resetProviderConversation(activeConversation.id, panelViewKey))
    ) {
      return;
    }
    setProvider(workspaceTargetKey, activeConversationId, nextProvider);
    setProviderMenuOpen(false);
  };

  const handleCreateConversation = async () => {
    if (
      !workspaceTargetKey ||
      !workspaceTarget.binding ||
      !canBeginAiConversation(workspaceTargetKey, tasks)
    ) {
      return;
    }
    if (
      activeConversation &&
      !runningConversationIdSet.has(activeConversation.id) &&
      !(await resetProviderConversation(activeConversation.id, panelViewKey))
    ) {
      return;
    }
    // 新会话可沿用当前目标的 Provider，但权限必须恢复为默认的“变更前确认”。
    beginConversation(workspaceTargetKey, currentTargetProvider, 'auto_safe');
    setInspectedConversationId(null);
    setHistoryOpen(false);
    setTaskMenuOpen(false);
    setProviderMenuOpen(false);
    clearCurrentAttachment();
    composerRef.current?.focus();
  };

  const handleClearConversation = async () => {
    if (!activeConversation || runningConversationIdSet.has(activeConversation.id)) return;
    if (!(await resetProviderConversation(activeConversation.id, panelViewKey))) return;
    clearMessages(activeConversation.id);
    clearConversationPresentation(activeConversation.id);
  };

  const handleSelectConversation = async (id: string) => {
    const selected = conversations.find((conversation) => conversation.id === id);
    if (!selected) return;
    if (
      activeConversation &&
      activeConversation.id !== id &&
      !runningConversationIdSet.has(activeConversation.id) &&
      !(await resetProviderConversation(activeConversation.id, panelViewKey))
    ) {
      return;
    }
    const selectedTargetKey = conversationTargetKey(selected);
    if (selectedTargetKey && selectedTargetKey === workspaceTargetKey) {
      selectConversation(selectedTargetKey, id);
      setInspectedConversationId(null);
      setAttachments((current) => ({ ...current, [selectedTargetKey]: null }));
    } else {
      setInspectedConversationId(id);
    }
    setTaskNavigationNotice(null);
    setHistoryOpen(false);
    setTaskMenuOpen(false);
  };

  const handleSelectTask = (task: AiTaskNavigationSource) => {
    const destination = resolveAiTaskNavigation(task, sessions, recentRemoteSessionIds);
    selectConversation(destination.targetKey, destination.conversationId);
    setHistoryOpen(false);
    setTaskMenuOpen(false);
    setProviderMenuOpen(false);

    if (destination.terminalSessionId !== null) {
      setInspectedConversationId(null);
      setTaskNavigationNotice(null);
      onActivateTerminal(destination.terminalSessionId);
      return;
    }

    // 来源 Tab 已关闭时仍允许查看和处理任务，但不能静默创建终端或重连。
    setInspectedConversationId(destination.conversationId);
    setTaskNavigationNotice({
      conversationId: destination.conversationId,
      message:
        task.target.kind === 'ssh'
          ? `${task.target.label} 的原终端标签已关闭，且当前没有该连接的其他终端；不会自动重连。`
          : `${task.target.label} 的原终端标签已关闭；不会自动新建终端。`,
    });
  };

  const handleDeleteConversation = async (id: string) => {
    if (runningConversationIdSet.has(id) || !(await resetProviderConversation(id, panelViewKey))) {
      return;
    }
    deleteConversation(id);
    clearConversationPresentation(id);
    if (inspectedConversationId === id) setInspectedConversationId(null);
  };

  const contextText = (() => {
    if (activeConversation && !viewingCurrentTarget) {
      return activeConversation.target
        ? `正在查看 ${activeConversation.target.label} 的会话 · 当前目标不匹配`
        : '正在查看未绑定历史 · 仅可查看';
    }
    if (activeView === 'sftp') return '文件会话 · 暂未接入 AI 执行上下文';
    if (activeSession && activeSessionStatus === 'connected') {
      return `已连接 ${activeSession.name}，可分析命令、日志与报错`;
    }
    if (activeSessionStatus === 'connecting') return '终端连接中，连接完成后可发送请求';
    if (activeSessionStatus === 'error') return '终端连接异常，请先恢复连接';
    return '连接终端后，可分析命令、日志与报错';
  })();

  return (
    <aside className={styles.panel} aria-label="AI 工作台" id="ai-panel">
      <header className={styles.header}>
        <div className={styles.titleGroup}>
          <h2>AI 助手</h2>
        </div>
        <div className={styles.headerActions}>
          <button
            aria-expanded={historyOpen}
            className={styles.iconButton}
            onClick={() => {
              setHistoryOpen((open) => !open);
              setTaskMenuOpen(false);
            }}
            title="历史会话"
            type="button"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M3 12a9 9 0 1 0 3-6.7M3 4v5h5M12 7v5l3 2" />
            </svg>
          </button>
          {taskAttentionCount ? (
            <button
              aria-expanded={taskMenuOpen}
              aria-label={`${taskAttentionCount} 个需要关注的 AI 任务`}
              className={`${styles.iconButton} ${styles.taskButton} ${
                backgroundTasks.some((task) => task.pendingApproval) ? styles.taskButtonWaiting : ''
              }`}
              onClick={() => {
                setTaskMenuOpen((open) => !open);
                setHistoryOpen(false);
              }}
              title="AI 任务"
              type="button"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M6 7h12M6 12h12M6 17h7" />
                <circle cx="18" cy="17" r="2" />
              </svg>
              <span className={styles.taskBadge}>{taskAttentionBadge}</span>
            </button>
          ) : null}
          <button
            aria-label="新建 AI 会话"
            className={styles.iconButton}
            disabled={!canBeginAiConversation(workspaceTargetKey, tasks)}
            onClick={() => void handleCreateConversation()}
            title="新建会话"
            type="button"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M12 5v14M5 12h14" />
            </svg>
          </button>
          <AiPanelToggle expanded onToggle={() => setOpen(false)} />
        </div>
      </header>

      {historyOpen ? (
        <AiHistoryMenu
          activeConversationId={activeConversationId}
          canClear={messages.length > 0}
          conversations={conversations}
          currentTargetKey={workspaceTargetKey}
          onClear={() => void handleClearConversation()}
          onDelete={(id) => void handleDeleteConversation(id)}
          onSelect={(id) => void handleSelectConversation(id)}
          runningConversationIds={runningConversationIds}
        />
      ) : null}

      {taskMenuOpen && taskAttentionCount ? (
        <AiTaskMenu
          activeConversationId={activeConversationId}
          completedResults={unreadTaskResults}
          onMarkAllRead={markAllTaskResultsRead}
          onSelect={handleSelectTask}
          runningTasks={backgroundTasks}
        />
      ) : null}

      <div className={styles.contextBar}>
        <span
          className={`${styles.contextStatusDot} ${
            activeConversation && !viewingCurrentTarget
              ? styles.contextStatusConnecting
              : activeView === 'sftp'
                ? styles.contextStatusInactive
                : activeSessionStatus === 'connected'
                  ? styles.contextStatusConnected
                  : activeSessionStatus === 'connecting'
                    ? styles.contextStatusConnecting
                    : activeSessionStatus === 'error'
                      ? styles.contextStatusError
                      : styles.contextStatusInactive
          }`}
        />
        <span>{contextText}</span>
      </div>

      <div className={styles.conversation}>
        <AiConversationView
          approvalSubmitting={activeTask?.approvalSubmitting ?? false}
          lastUserMessageId={lastUserMessageId}
          messages={messages}
          notice={visibleNotice}
          onResolveApproval={(approved) => {
            if (activeTask) resolveApproval(activeTask.sessionId, approved);
          }}
          onRetry={retryLastQuestion}
          pendingApproval={activeTask?.pendingApproval ?? null}
          providerName={selectedProvider.name}
          runParts={activeTask?.runParts ?? []}
          running={Boolean(activeTask)}
          streamText={activeTask?.streamText ?? ''}
        />
      </div>

      <AiComposer
        attachment={attachment}
        attachmentLoading={attachmentLoading}
        commandPolicy={commandPolicy}
        composerDisabled={
          !viewingCurrentTarget || !workspaceTargetKey || currentTargetBusyElsewhere
        }
        composerRef={composerRef}
        controlsDisabled={Boolean(currentTargetTask)}
        draft={draft}
        fileInputRef={fileInputRef}
        onAttachmentChange={handleAttachmentChange}
        onAttachmentRemove={clearCurrentAttachment}
        onCommandPolicyChange={(policy) => {
          if (workspaceTargetKey) {
            setCommandPolicy(workspaceTargetKey, activeConversationId, policy);
          }
        }}
        onComposerKeyDown={handleComposerKeyDown}
        onDraftChange={handleDraftChange}
        onProviderChange={(nextProvider) => void handleProviderChange(nextProvider)}
        onProviderMenuToggle={() => setProviderMenuOpen((open) => !open)}
        onStop={() => {
          if (activeTask) requestSessionStop(activeTask.sessionId);
        }}
        onSubmit={submit}
        provider={selectedProvider}
        providerAvailable={providerAvailability[provider]}
        providerId={provider}
        providerMenuOpen={providerMenuOpen}
        running={Boolean(activeTask)}
        statusMessage={
          currentTargetBusyElsewhere
            ? '当前终端的另一会话正在执行，请从历史会话中返回任务。'
            : !viewingCurrentTarget || !targetReady
              ? targetUnavailableMessage
              : undefined
        }
        targetReady={targetReady}
        targetUnavailableMessage={targetUnavailableMessage}
      />
    </aside>
  );
}

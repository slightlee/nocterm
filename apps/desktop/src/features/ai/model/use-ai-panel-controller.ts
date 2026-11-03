//! AI 面板控制器：把 AiPanel 渲染层之下的业务编排（目标解析、会话选择、
//! 提交/重试、输入历史导航、附件、Provider 切换、任务导航）收拢为一个 Hook。
//!
//! 职责边界：本 Hook 只做状态编排与行为回调，不持有 JSX；渲染层通过返回值
//! 消费状态与处理器。全部逻辑与既有 AiPanel 内联实现一一对应，行为零变化。

import { useEffect, useMemo, useRef, useState } from 'react';

import type { TerminalSession, TerminalSessionId, TerminalStatus } from '../../terminal';
import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import { resetAiConversation } from '../api/ai-client';
import { useAiPersistence } from '../hooks/use-ai-persistence';
import { useAiRuntime, type AiRuntimePresentation } from '../hooks/use-ai-runtime';
import {
  buildAiVisibleContextSummary,
  createTerminalSelectionAttachment,
  EMPTY_AI_COMPOSER_ATTACHMENTS,
  listAiAttachments,
  readAiAttachment,
  removeAiAttachment,
  setAiAttachment,
  stripAiVisibleContextSummary,
  type AiAttachment,
  type AiComposerAttachments,
} from '../model/ai-attachment';
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
  type AiConversation,
} from '../model/ai-store';
import { canBeginAiConversation, resolveAiTaskNavigation } from '../model/ai-panel-state';
import {
  findAiRuntimeTaskByConversation,
  findAiRuntimeTaskByTarget,
  listBackgroundAiRuntimeTasks,
  listCompletedAiTaskResults,
  type AiCompletedTaskResult,
  type AiRuntimeTask,
  type AiTaskNavigationSource,
} from '../model/ai-runtime-task';
import {
  aiConversationTargetKey,
  aiConversationTargetsEqual,
  isAiTerminalTargetReady,
  resolveAiWorkspaceTarget,
  type AiWorkspaceView,
} from '../model/ai-target';
import {
  AI_PROVIDERS,
  type AiCommandPolicy,
  type AiMessage,
  type AiProviderId,
} from '../model/ai-types';
import {
  resolveAiContextBar,
  resolveFallbackQuestion,
  resolveTargetUnavailableMessage,
  resolveVisibleNotice,
} from './ai-panel-view-model';

/** 控制器对外暴露的视图状态与处理器集合；AiPanel 渲染层按需消费。 */
export interface AiPanelController {
  /** 当前查看的会话：检视中的历史会话优先于当前目标会话。 */
  activeConversation: AiConversation | null;
  activeConversationId: string | null;
  activePresentation: AiRuntimePresentation | undefined;
  activeSessionStatus: TerminalStatus | 'idle';
  activeTask: AiRuntimeTask | null;
  attachmentItems: AiAttachment[];
  attachmentLoading: boolean;
  backgroundTasks: AiRuntimeTask[];
  composerAttachments: AiComposerAttachments;
  composerRef: React.RefObject<HTMLTextAreaElement | null>;
  commandPolicy: AiCommandPolicy;
  composerState: { draft: string; provider: AiProviderId; commandPolicy: AiCommandPolicy };
  /** 新建会话按钮可用性：目标已绑定且该目标没有运行中的任务。 */
  canBeginConversation: boolean;
  conversations: AiConversation[];
  contextText: string;
  currentTargetBusyElsewhere: boolean;
  currentTargetTask: AiRuntimeTask | null;
  draft: string;
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  globalNotice: string | null;
  historyOpen: boolean;
  inspectedConversationId: string | null;
  lastUserMessageId: string | undefined;
  messages: AiMessage[];
  panelViewKey: string;
  provider: AiProviderId;
  providerAvailability: Record<string, boolean>;
  providerMenuOpen: boolean;
  selectedProvider: (typeof AI_PROVIDERS)[number];
  targetConversation: AiConversation | null;
  targetReady: boolean;
  targetUnavailableMessage: string;
  taskAttentionBadge: number | '99+';
  taskAttentionCount: number;
  taskMenuOpen: boolean;
  unreadTaskResults: AiCompletedTaskResult[];
  viewingCurrentTarget: boolean;
  visibleNotice: string | null;
  workspaceTarget: ReturnType<typeof resolveAiWorkspaceTarget>;
  workspaceTargetKey: string | null;
  handleAttachmentChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  handleClearConversation: () => Promise<void>;
  handleComposerKeyDown: (event: React.KeyboardEvent<HTMLTextAreaElement>) => void;
  handleCreateConversation: () => Promise<void>;
  handleDeleteConversation: (id: string) => Promise<void>;
  handleDraftChange: (value: string) => void;
  handleProviderChange: (nextProvider: AiProviderId) => Promise<void>;
  handleSelectConversation: (id: string) => Promise<void>;
  handleSelectTask: (task: AiTaskNavigationSource) => void;
  removeCurrentAttachment: (kind: AiAttachment['kind']) => void;
  markAllTaskResultsRead: ReturnType<typeof useAiRuntime>['markAllTaskResultsRead'];
  requestSessionStop: ReturnType<typeof useAiRuntime>['requestSessionStop'];
  runningConversationIds: string[];
  resolveApproval: ReturnType<typeof useAiRuntime>['resolveApproval'];
  retryLastQuestion: () => void;
  /** 关闭 AI 面板（AiPanelToggle 调用）。 */
  setOpen: (open: boolean) => void;
  /** 设置当前目标的命令权限（AiComposer 权限选择器调用）。 */
  setCommandPolicy: ReturnType<typeof useAiStore.getState>['setCommandPolicy'];
  setHistoryOpen: (open: boolean) => void;
  setProviderMenuOpen: (open: boolean) => void;
  setTaskMenuOpen: (open: boolean) => void;
  submit: (event?: React.FormEvent) => void;
}

/** 终端选区加入 AI 的请求载荷；由 AppShell 捕获选区后投递。 */
export interface AiTerminalSelectionRequest {
  id: string;
  terminalSessionId: TerminalSessionId;
  selection: string;
}

/** 控制器必需的外部输入：AiPanel 从父级与终端 store 透传。 */
export interface AiPanelControllerInput {
  activeId: TerminalSessionId | null;
  activeView: AiWorkspaceView;
  onActivateTerminal: (sessionId: TerminalSessionId) => void;
  onTerminalSelectionHandled: (requestId: string) => void;
  recentRemoteSessionIds: Readonly<Record<string, readonly string[]>>;
  sessions: TerminalSession[];
  terminalSelectionRequest: AiTerminalSelectionRequest | null;
  terminalStatuses: Record<string, TerminalStatus>;
  visible: boolean;
}

/** AI 工作台控制器：常驻接收任务事件；Tab 切换只选择目标视图，不创建空会话。 */
export function useAiPanelController(input: AiPanelControllerInput): AiPanelController {
  const {
    activeId,
    activeView,
    onActivateTerminal,
    onTerminalSelectionHandled,
    recentRemoteSessionIds,
    sessions,
    terminalSelectionRequest,
    terminalStatuses,
    visible,
  } = input;

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
  const [attachments, setAttachments] = useState<Record<string, AiComposerAttachments>>({});
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
  const commandPolicy = activeConversation?.commandPolicy ?? composerState.commandPolicy;
  const currentTargetProvider = targetConversation?.provider ?? composerState.provider;
  const selectedProvider = AI_PROVIDERS.find((item) => item.id === provider) ?? AI_PROVIDERS[0];
  const messages = activeConversation?.messages ?? [];
  const draft = inspectedConversation ? '' : composerState.draft;
  const composerAttachments =
    workspaceTargetKey && !inspectedConversation
      ? (attachments[workspaceTargetKey] ?? EMPTY_AI_COMPOSER_ATTACHMENTS)
      : EMPTY_AI_COMPOSER_ATTACHMENTS;
  const attachmentItems = listAiAttachments(composerAttachments);
  const attachmentLoading = Boolean(
    workspaceTargetKey && attachmentLoadingTargetKey === workspaceTargetKey
  );
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
  const visibleNotice = resolveVisibleNotice({
    activeConversationId,
    activeTaskNotice: activeTask?.notice,
    activePresentationNotice: activePresentation?.notice,
    globalNotice,
    panelNotice,
    taskNavigationNotice,
  });
  const canBeginConversation = Boolean(
    workspaceTargetKey && canBeginAiConversation(workspaceTargetKey, tasks)
  );
  const lastUserMessageId = activePresentation?.canRetryQuestion
    ? [...messages].reverse().find((message) => message.role === 'user')?.id
    : undefined;

  useEffect(() => {
    // 同一 SSH 连接的多个终端 Tab 共享 target key，因此不会误清其会话视图。
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
    setTaskMenuOpen(false);
  }, [taskAttentionCount]);

  useEffect(() => {
    if (!terminalSelectionRequest) return;
    const request = terminalSelectionRequest;
    if (String(activeId) !== String(request.terminalSessionId) || !workspaceTargetKey) {
      onTerminalSelectionHandled(request.id);
      return;
    }

    try {
      const nextAttachment = createTerminalSelectionAttachment(request.selection);
      setInspectedConversationId(null);
      setAttachments((current) => ({
        ...current,
        [workspaceTargetKey]: setAiAttachment(
          current[workspaceTargetKey] ?? EMPTY_AI_COMPOSER_ATTACHMENTS,
          nextAttachment
        ),
      }));
      setPanelNotices((current) => ({ ...current, [workspaceTargetKey]: null }));
      requestAnimationFrame(() => composerRef.current?.focus());
    } catch (error: unknown) {
      setPanelNotices((current) => ({
        ...current,
        [workspaceTargetKey]: error instanceof Error ? error.message : '读取终端选区失败。',
      }));
    } finally {
      onTerminalSelectionHandled(request.id);
    }
  }, [activeId, onTerminalSelectionHandled, terminalSelectionRequest, workspaceTargetKey]);

  const targetUnavailableMessage = resolveTargetUnavailableMessage({
    activeConversation,
    activeSessionStatus,
    activeView,
    hasBinding: Boolean(workspaceTarget.binding),
    viewingCurrentTarget,
  });

  const clearCurrentAttachments = () => {
    if (!workspaceTargetKey) return;
    setAttachments((current) => ({
      ...current,
      [workspaceTargetKey]: EMPTY_AI_COMPOSER_ATTACHMENTS,
    }));
  };

  const removeCurrentAttachment = (kind: AiAttachment['kind']) => {
    if (!workspaceTargetKey) return;
    setAttachments((current) => {
      const existing = current[workspaceTargetKey] ?? EMPTY_AI_COMPOSER_ATTACHMENTS;
      return {
        ...current,
        [workspaceTargetKey]: removeAiAttachment(existing, kind),
      };
    });
  };

  /** 异步操作必须把结果写回发起操作的目标，不能跟随之后切换到的 Tab。 */
  const setTargetPanelNotice = (targetKey: string, message: string | null) => {
    setPanelNotices((current) => ({ ...current, [targetKey]: message }));
  };

  const submit = (event?: React.FormEvent) => {
    event?.preventDefault();
    if (currentTargetTask || !workspaceTargetKey || !workspaceTarget.binding) return;
    const content = draft.trim();
    if (!content && attachmentItems.length === 0) return;
    if (!targetReady) {
      setTargetPanelNotice(panelViewKey, `${targetUnavailableMessage}，暂时无法发送新请求。`);
      return;
    }

    const selectedAttachments = attachmentItems;
    const selectedFile = composerAttachments.file;
    const selectedTerminal = composerAttachments.terminalSelection;
    const question =
      content ||
      resolveFallbackQuestion({
        hasTerminalSelection: Boolean(selectedTerminal),
        fileName: selectedFile?.name ?? null,
      });
    const visibleContextSummary = buildAiVisibleContextSummary(selectedAttachments);
    const visibleMessage = visibleContextSummary
      ? `${question}\n\n${visibleContextSummary}`
      : question;
    const conversationId =
      activeConversation?.id ??
      createConversation(workspaceTarget.binding, provider, commandPolicy);
    const history = activeConversation?.messages ?? [];
    addMessage(conversationId, 'user', visibleMessage);
    setDraft(workspaceTargetKey, '');
    clearCurrentAttachments();
    setTargetPanelNotice(panelViewKey, null);
    sendQuestion({
      question,
      attachments: selectedAttachments,
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
    const question = stripAiVisibleContextSummary(retried.content);
    const retryAttachments = activePresentation?.retryAttachments ?? [];
    const visibleContextSummary = buildAiVisibleContextSummary(retryAttachments);
    const visibleMessage = visibleContextSummary
      ? `${question}\n\n${visibleContextSummary}`
      : question;
    messages
      .slice(lastUserIndex)
      .forEach((message) => deleteMessage(activeConversation.id, message.id));
    addMessage(activeConversation.id, 'user', visibleMessage);
    sendQuestion({
      question,
      attachments: retryAttachments,
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

  const handleComposerKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
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

  const handleAttachmentChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file || !workspaceTargetKey || !viewingCurrentTarget) return;

    const targetKey = workspaceTargetKey;
    setAttachmentLoadingTargetKey(targetKey);
    void readAiAttachment(file)
      .then((nextAttachment) => {
        setAttachments((current) => ({
          ...current,
          [targetKey]: setAiAttachment(
            current[targetKey] ?? EMPTY_AI_COMPOSER_ATTACHMENTS,
            nextAttachment
          ),
        }));
        setTargetPanelNotice(targetKey, null);
      })
      .catch((error: unknown) => {
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

  const handleProviderChange = async (nextProvider: Parameters<typeof setProvider>[2]) => {
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
    clearCurrentAttachments();
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
      setAttachments((current) => ({
        ...current,
        [selectedTargetKey]: EMPTY_AI_COMPOSER_ATTACHMENTS,
      }));
    } else {
      setInspectedConversationId(id);
    }
    setTaskNavigationNotice(null);
    setHistoryOpen(false);
    setTaskMenuOpen(false);
  };

  const handleSelectTask = (task: Parameters<typeof resolveAiTaskNavigation>[0]) => {
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

  const contextText = resolveAiContextBar({
    activeSession,
    activeSessionStatus,
    activeView,
    activeConversation,
    viewingCurrentTarget,
  }).text;

  return {
    activeConversation,
    activeConversationId,
    activePresentation,
    activeSessionStatus,
    activeTask,
    attachmentItems,
    attachmentLoading,
    backgroundTasks,
    commandPolicy,
    canBeginConversation,
    composerAttachments,
    composerRef,
    conversations,
    composerState,
    contextText,
    currentTargetBusyElsewhere,
    currentTargetTask,
    draft,
    fileInputRef,
    globalNotice,
    historyOpen,
    inspectedConversationId,
    lastUserMessageId,
    messages,
    panelViewKey,
    provider,
    providerAvailability,
    providerMenuOpen,
    selectedProvider,
    targetConversation,
    targetReady,
    targetUnavailableMessage,
    taskAttentionBadge,
    taskAttentionCount,
    taskMenuOpen,
    unreadTaskResults,
    viewingCurrentTarget,
    visibleNotice,
    workspaceTarget,
    workspaceTargetKey,
    handleAttachmentChange,
    handleClearConversation,
    handleComposerKeyDown,
    handleCreateConversation,
    handleDeleteConversation,
    handleDraftChange,
    handleProviderChange,
    handleSelectConversation,
    handleSelectTask,
    removeCurrentAttachment,
    markAllTaskResultsRead,
    requestSessionStop,
    resolveApproval,
    runningConversationIds,
    setHistoryOpen,
    setProviderMenuOpen,
    setTaskMenuOpen,
    submit,
    retryLastQuestion,
    setOpen,
    setCommandPolicy,
  };
}

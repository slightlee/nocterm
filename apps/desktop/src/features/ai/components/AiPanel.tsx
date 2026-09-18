import {
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type FormEvent,
  type KeyboardEvent,
} from 'react';

import { useTerminalStore } from '../../terminal';
import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import { resetAiConversation } from '../api/ai-client';
import { useAiRuntime } from '../hooks/use-ai-runtime';
import { useAiStore } from '../model/ai-store';
import { readAiAttachment, type AiAttachment } from '../model/ai-attachment';
import { isAiTerminalTargetReady, resolveAiTerminalTarget } from '../model/ai-target';
import { AI_PROVIDERS, type AiCommandPolicy, type AiProviderId } from '../model/ai-types';
import { AiComposer } from './AiComposer';
import { AiConversationView } from './AiConversationView';
import { AiHistoryMenu } from './AiHistoryMenu';
import styles from './AiPanel.module.css';

interface AiPanelProps {
  visible: boolean;
}

/** AI 工作台是 Agent 的宿主界面；组件常驻，折叠或切页不会丢失任务事件。 */
export function AiPanel({ visible }: AiPanelProps) {
  const sessions = useTerminalStore((state) => state.sessions);
  const activeId = useTerminalStore((state) => state.activeId);
  const terminalStatuses = useTerminalStore((state) => state.statuses);
  const conversations = useAiStore((state) => state.conversations);
  const activeConversationId = useAiStore((state) => state.activeConversationId);
  const draft = useAiStore((state) => state.draft);
  const setOpen = useAiStore((state) => state.setOpen);
  const setProvider = useAiStore((state) => state.setProvider);
  const setCommandPolicy = useAiStore((state) => state.setCommandPolicy);
  const setDraft = useAiStore((state) => state.setDraft);
  const addMessage = useAiStore((state) => state.addMessage);
  const deleteMessage = useAiStore((state) => state.deleteMessage);
  const clearMessages = useAiStore((state) => state.clearMessages);
  const createConversation = useAiStore((state) => state.createConversation);
  const deleteConversation = useAiStore((state) => state.deleteConversation);
  const selectConversation = useAiStore((state) => state.selectConversation);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [providerMenuOpen, setProviderMenuOpen] = useState(false);
  const [attachment, setAttachment] = useState<AiAttachment | null>(null);
  const [attachmentLoading, setAttachmentLoading] = useState(false);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const activeSession = useMemo(
    () => sessions.find((session) => session.id === activeId) ?? null,
    [activeId, sessions]
  );
  // 目标绑定必须以终端后端确认的 connected 为准；标签存在不等于 SSH/PTY 已认证。
  const activeSessionStatus = activeSession
    ? (terminalStatuses[String(activeSession.id)] ?? 'connecting')
    : 'idle';
  const terminalTarget = useMemo(
    () => resolveAiTerminalTarget(activeSession, activeSessionStatus),
    [activeSession, activeSessionStatus]
  );
  const terminalReady = isAiTerminalTargetReady(terminalTarget);
  const activeConversation =
    conversations.find((item) => item.id === activeConversationId) ?? conversations[0];
  const conversationId = activeConversation?.id ?? activeConversationId;
  const provider = activeConversation?.provider ?? 'codex';
  const commandPolicy: AiCommandPolicy = activeConversation?.commandPolicy ?? 'auto_safe';
  const selectedProvider = AI_PROVIDERS.find((item) => item.id === provider) ?? AI_PROVIDERS[0];
  const messages = activeConversation?.messages ?? [];
  const {
    notice,
    setNotice,
    canRetryQuestion,
    runningSessionId,
    streamText,
    runParts,
    pendingApproval,
    approvalSubmitting,
    providerAvailability,
    sendQuestion,
    resolveApproval,
    requestSessionStop,
    resetPresentation,
  } = useAiRuntime({ addMessage });
  // 失败提示跟随最后一条提问展示，让“重新发送”与被重发的消息在视觉上绑定。
  const lastUserMessageId = canRetryQuestion
    ? [...messages].reverse().find((message) => message.role === 'user')?.id
    : undefined;

  if (!visible) return null;
  const submit = (event?: FormEvent) => {
    event?.preventDefault();
    if (runningSessionId) return;
    const content = draft.trim();
    if (!content && !attachment) return;
    if (!terminalReady) {
      setNotice('请先连接本地终端或远程服务器，再使用 AI 助手。');
      return;
    }
    const selectedAttachment = attachment;
    const question = content || `请分析附件：${selectedAttachment?.name ?? '未命名文件'}`;
    const visibleMessage = selectedAttachment
      ? `${question}\n\n附件：${selectedAttachment.name}`
      : question;
    addMessage('user', visibleMessage);
    setDraft('');
    setAttachment(null);
    sendQuestion({
      question,
      attachment: selectedAttachment,
      history: messages,
      conversationId,
      provider,
      target: terminalTarget,
      commandPolicy,
    });
  };

  /** 失败重试：撤销本轮消息（最后一条提问及其后的残留输出），再按原话重新发送。 */
  const retryLastQuestion = () => {
    if (runningSessionId) return;
    let lastUserIndex = -1;
    for (let i = messages.length - 1; i >= 0; i -= 1) {
      if (messages[i]?.role === 'user') {
        lastUserIndex = i;
        break;
      }
    }
    const retried = lastUserIndex >= 0 ? messages[lastUserIndex] : undefined;
    if (!retried) return;
    if (!terminalReady) {
      setNotice('请先连接本地终端或远程服务器，再重新发送。');
      return;
    }
    // 附件正文不落盘、无法随重试重读，只发送文本部分并去掉“附件：名称”标记。
    const question = retried.content.replace(/\n\n附件：[^\n]*$/, '');
    messages.slice(lastUserIndex).forEach((message) => deleteMessage(message.id));
    addMessage('user', question);
    sendQuestion({
      question,
      attachment: null,
      history: messages.slice(0, lastUserIndex),
      conversationId,
      provider,
      target: terminalTarget,
      commandPolicy,
    });
  };

  const handleComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault();
      submit();
    }
  };

  const handleAttachmentChange = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file) return;

    setAttachmentLoading(true);
    void readAiAttachment(file)
      .then((nextAttachment) => {
        setAttachment(nextAttachment);
        setNotice(null);
      })
      .catch((error: unknown) => {
        setAttachment(null);
        setNotice(error instanceof Error ? error.message : '读取附件失败。');
      })
      .finally(() => setAttachmentLoading(false));
  };

  // 浏览器预览没有 Tauri IPC；会话 UI 仍可操作，Provider 运行时只在桌面端重置。
  const resetProviderConversation = async (id: string): Promise<boolean> => {
    if (!isDesktopRuntime()) return true;
    try {
      await resetAiConversation(id);
      return true;
    } catch (error: unknown) {
      setNotice(error instanceof Error ? error.message : '重置 AI 会话失败。');
      return false;
    }
  };

  const handleProviderChange = async (nextProvider: AiProviderId) => {
    if (nextProvider === provider) {
      setProviderMenuOpen(false);
      return;
    }
    if (!(await resetProviderConversation(conversationId))) return;
    setProvider(nextProvider);
    setProviderMenuOpen(false);
  };

  const handleCreateConversation = async () => {
    if (runningSessionId) return;
    if (!(await resetProviderConversation(conversationId))) return;
    createConversation();
    setHistoryOpen(false);
    setProviderMenuOpen(false);
    setAttachment(null);
    resetPresentation();
    composerRef.current?.focus();
  };

  const handleClearConversation = async () => {
    if (!(await resetProviderConversation(conversationId))) return;
    clearMessages();
  };

  const handleSelectConversation = async (id: string) => {
    if (id !== conversationId && !(await resetProviderConversation(conversationId))) return;
    selectConversation(id);
    setAttachment(null);
    setHistoryOpen(false);
  };

  const handleDeleteConversation = async (id: string) => {
    if (!(await resetProviderConversation(id))) return;
    deleteConversation(id);
    setAttachment(null);
  };

  return (
    <aside className={styles.panel} aria-label="AI 工作台">
      <header className={styles.header}>
        <div className={styles.titleGroup}>
          <h2>AI 助手</h2>
        </div>
        <div className={styles.headerActions}>
          <button
            aria-expanded={historyOpen}
            className={styles.iconButton}
            onClick={() => setHistoryOpen((open) => !open)}
            title="历史会话"
            type="button"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M3 12a9 9 0 1 0 3-6.7M3 4v5h5M12 7v5l3 2" />
            </svg>
          </button>
          <button
            aria-label="新建 AI 会话"
            className={styles.iconButton}
            disabled={Boolean(runningSessionId)}
            onClick={handleCreateConversation}
            title="新建会话"
            type="button"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M12 5v14M5 12h14" />
            </svg>
          </button>
        </div>
      </header>

      <button
        aria-label="折叠 AI 助手"
        className={styles.collapseButton}
        onClick={() => setOpen(false)}
        title="折叠 AI 助手"
        type="button"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="m10 7 5 5-5 5" />
        </svg>
      </button>

      {historyOpen ? (
        <AiHistoryMenu
          activeConversationId={activeConversationId}
          canClear={messages.length > 0}
          conversations={conversations}
          disabled={Boolean(runningSessionId)}
          onClear={() => void handleClearConversation()}
          onDelete={(id) => void handleDeleteConversation(id)}
          onSelect={(id) => void handleSelectConversation(id)}
        />
      ) : null}

      <div className={styles.contextBar}>
        <span
          className={`${styles.contextStatusDot} ${
            activeSessionStatus === 'connected'
              ? styles.contextStatusConnected
              : activeSessionStatus === 'connecting'
                ? styles.contextStatusConnecting
                : activeSessionStatus === 'error'
                  ? styles.contextStatusError
                  : styles.contextStatusInactive
          }`}
        />
        <span>
          {activeSession && activeSessionStatus === 'connected'
            ? `已连接 ${activeSession.name}，可分析命令、日志与报错`
            : activeSessionStatus === 'connecting'
              ? '终端连接中，连接完成后可分析命令、日志与报错'
              : activeSessionStatus === 'error'
                ? '终端连接异常，请先恢复连接后再让 AI 执行操作'
                : '连接终端后，可分析命令、日志与报错'}
        </span>
      </div>

      <div className={styles.conversation}>
        <AiConversationView
          approvalSubmitting={approvalSubmitting}
          lastUserMessageId={lastUserMessageId}
          messages={messages}
          notice={notice}
          onResolveApproval={resolveApproval}
          onRetry={retryLastQuestion}
          pendingApproval={pendingApproval}
          providerName={selectedProvider.name}
          runParts={runParts}
          running={Boolean(runningSessionId)}
          streamText={streamText}
        />
      </div>

      <AiComposer
        attachment={attachment}
        attachmentLoading={attachmentLoading}
        terminalReady={terminalReady}
        composerRef={composerRef}
        draft={draft}
        fileInputRef={fileInputRef}
        onAttachmentChange={handleAttachmentChange}
        onAttachmentRemove={() => setAttachment(null)}
        onComposerKeyDown={handleComposerKeyDown}
        onDraftChange={setDraft}
        onProviderChange={handleProviderChange}
        commandPolicy={commandPolicy}
        onCommandPolicyChange={setCommandPolicy}
        onProviderMenuToggle={() => setProviderMenuOpen((open) => !open)}
        onStop={() => {
          if (runningSessionId) requestSessionStop(runningSessionId);
        }}
        onSubmit={submit}
        provider={selectedProvider}
        providerAvailable={providerAvailability[provider]}
        providerId={provider}
        providerMenuOpen={providerMenuOpen}
        runningSessionId={runningSessionId}
      />
    </aside>
  );
}

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type FormEvent,
  type KeyboardEvent,
} from 'react';

import { useTerminalStore } from '../../terminal';
import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import {
  getAiProviderStatus,
  onAiExit,
  onAiOutput,
  onAiToolApproval,
  onAiToolApprovalClosed,
  resetAiConversation,
  resolveAiToolApproval,
  startAiSession,
  stopAiSession,
} from '../api/ai-client';
import { useAiStore } from '../model/ai-store';
import { buildAiPrompt, readAiAttachment, type AiAttachment } from '../model/ai-attachment';
import { buildConversationContext } from '../model/ai-conversation';
import { resolveAiTerminalTarget } from '../model/ai-target';
import {
  extractAiActivities,
  extractAiStreamDelta,
  extractAiText,
  isAiFullTextEvent,
} from '../model/ai-output';
import {
  AI_PROVIDERS,
  type AiCommandPolicy,
  type AiMessagePart,
  type AiProviderId,
  type AiToolApprovalEvent,
} from '../model/ai-types';
import { AiComposer } from './AiComposer';
import { AiConversationView } from './AiConversationView';
import styles from './AiPanel.module.css';

// MCP 握手或 Provider 网络请求异常时，不能让面板无限显示“输出中”。
const AI_IDLE_TIMEOUT_MS = 90_000;
const AI_IDLE_TIMEOUT_SECONDS = AI_IDLE_TIMEOUT_MS / 1_000;

function appendTextPart(parts: AiMessagePart[], text: string): AiMessagePart[] {
  const last = parts.at(-1);
  if (last?.type === 'text') {
    return [...parts.slice(0, -1), { ...last, content: last.content + text }];
  }
  return [...parts, { type: 'text', content: text }];
}

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
  const [notice, setNotice] = useState<string | null>(null);
  const [canRetryQuestion, setCanRetryQuestion] = useState(false);
  const [runningSessionId, setRunningSessionId] = useState<string | null>(null);
  const [streamText, setStreamText] = useState('');
  const [runParts, setRunParts] = useState<AiMessagePart[]>([]);
  const [liveThinking, setLiveThinking] = useState('');
  const [pendingApproval, setPendingApproval] = useState<AiToolApprovalEvent | null>(null);
  const [approvalSubmitting, setApprovalSubmitting] = useState(false);
  const [providerAvailability, setProviderAvailability] = useState<Record<string, boolean>>({});
  const [eventListenersReady, setEventListenersReady] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [providerMenuOpen, setProviderMenuOpen] = useState(false);
  const [attachment, setAttachment] = useState<AiAttachment | null>(null);
  const [attachmentLoading, setAttachmentLoading] = useState(false);
  const runningSessionRef = useRef<string | null>(null);
  const streamTextRef = useRef('');
  const runPartsRef = useRef<AiMessagePart[]>([]);
  const stderrLastRef = useRef('');
  const answerStreamedRef = useRef(false);
  const timeoutRef = useRef<number | null>(null);
  const approvalExpiryTimeoutRef = useRef<number | null>(null);
  const timedOutSessionRef = useRef<string | null>(null);
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
  const activeConversation =
    conversations.find((item) => item.id === activeConversationId) ?? conversations[0];
  const conversationId = activeConversation?.id ?? activeConversationId;
  const provider = activeConversation?.provider ?? 'codex';
  const commandPolicy: AiCommandPolicy = activeConversation?.commandPolicy ?? 'auto_safe';
  const selectedProvider = AI_PROVIDERS.find((item) => item.id === provider) ?? AI_PROVIDERS[0];
  const messages = activeConversation?.messages ?? [];
  // 失败提示跟随最后一条提问展示，让“重新发送”与被重发的消息在视觉上绑定。
  const lastUserMessageId = canRetryQuestion
    ? [...messages].reverse().find((message) => message.role === 'user')?.id
    : undefined;

  /** 所有停止路径都必须消费 IPC 失败，避免按钮无响应或浏览器产生未处理 Promise。 */
  const requestSessionStop = useCallback((sessionId: string) => {
    void stopAiSession(sessionId).catch((error: unknown) => {
      if (runningSessionRef.current !== sessionId) return;
      timedOutSessionRef.current = null;
      setNotice(error instanceof Error ? error.message : '停止 AI 会话失败。');
      setCanRetryQuestion(false);
    });
  }, []);

  useEffect(() => {
    runningSessionRef.current = runningSessionId;
  }, [runningSessionId]);

  useEffect(() => {
    streamTextRef.current = streamText;
  }, [streamText]);

  useEffect(() => {
    if (!isDesktopRuntime()) return;
    let disposed = false;
    const outputPromise = onAiOutput((event) => {
      if (disposed || event.sessionId !== runningSessionRef.current) return;
      // 有持续输出说明 Provider 仍在工作；只限制无任何进展的卡死状态。
      if (timeoutRef.current !== null) window.clearTimeout(timeoutRef.current);
      timeoutRef.current = window.setTimeout(() => {
        if (runningSessionRef.current !== event.sessionId) return;
        timedOutSessionRef.current = event.sessionId;
        requestSessionStop(event.sessionId);
      }, AI_IDLE_TIMEOUT_MS);
      // stderr 是人类可读诊断信息，只保留末行供失败提示使用，不混入助手输出流。
      if (event.stream === 'stderr') {
        const line = event.data.trim();
        if (line) stderrLastRef.current = line;
        return;
      }
      // 增量事件：回答逐字流出、思考实时滚动，等待期不再空白。
      const delta = extractAiStreamDelta(event.data);
      if (delta?.answer) {
        answerStreamedRef.current = true;
        setLiveThinking('');
        const nextText = streamTextRef.current + delta.answer;
        const nextParts = appendTextPart(runPartsRef.current, delta.answer);
        // Provider 可能在 React 提交下一次渲染前退出，ref 必须先于状态同步更新。
        streamTextRef.current = nextText;
        runPartsRef.current = nextParts;
        setStreamText(nextText);
        setRunParts(nextParts);
        return;
      }
      if (delta?.thinking) {
        setLiveThinking((current) => (current + delta.thinking).slice(-160));
        return;
      }
      if (delta) return;
      // 过程信息（思考、工具调用）实时展示，让长时间执行不再是一片空白。
      const foundActivities = extractAiActivities(event.data);
      if (foundActivities.length > 0) {
        const nextParts = [
          ...runPartsRef.current,
          ...foundActivities.map((activity): AiMessagePart => ({
            type: 'activity',
            kind: activity.kind,
            content: activity.text,
          })),
        ];
        runPartsRef.current = nextParts;
        setRunParts(nextParts);
      }
      // 消息级事件到达说明这一轮思考已经固化成条目，滚动的实时思考行可以退场。
      if (isAiFullTextEvent(event.data)) {
        setLiveThinking('');
      }
      const text = extractAiText(event.data);
      if (!text) return;
      // 回答已通过增量流出时，末尾的整段 assistant/result 事件不再重复。
      if (answerStreamedRef.current && isAiFullTextEvent(event.data)) return;
      const nextText = `${streamTextRef.current}${text}\n`;
      const nextParts = appendTextPart(runPartsRef.current, `${text}\n`);
      streamTextRef.current = nextText;
      runPartsRef.current = nextParts;
      setStreamText(nextText);
      setRunParts(nextParts);
    });
    const exitPromise = onAiExit((event) => {
      if (disposed || event.sessionId !== runningSessionRef.current) return;
      if (timeoutRef.current !== null) {
        window.clearTimeout(timeoutRef.current);
        timeoutRef.current = null;
      }
      setRunningSessionId(null);
      setPendingApproval(null);
      setApprovalSubmitting(false);
      if (approvalExpiryTimeoutRef.current !== null) {
        window.clearTimeout(approvalExpiryTimeoutRef.current);
        approvalExpiryTimeoutRef.current = null;
      }
      runningSessionRef.current = null;
      if (streamTextRef.current.trim()) {
        addMessage('assistant', streamTextRef.current.trim(), runPartsRef.current);
      }
      streamTextRef.current = '';
      runPartsRef.current = [];
      setStreamText('');
      setRunParts([]);
      if (timedOutSessionRef.current === event.sessionId) {
        timedOutSessionRef.current = null;
        setNotice(
          `AI 会话超过 ${AI_IDLE_TIMEOUT_SECONDS} 秒没有新的输出，已自动停止。请检查 MCP/Provider 状态后重试。`
        );
        setCanRetryQuestion(true);
      } else if (event.cancelled) {
        setNotice('AI 会话已停止。');
        setCanRetryQuestion(true);
      } else if (event.code !== 0) {
        // 附带 stderr 末行，让参数缺失、未登录等真实原因直接可见，而不是只有退出码。
        const detail = stderrLastRef.current.slice(0, 200);
        setNotice(
          `AI Provider 异常退出（代码 ${event.code ?? '未知'}）${detail ? `：${detail}` : ''}。`
        );
        setCanRetryQuestion(true);
      } else {
        setNotice(null);
        setCanRetryQuestion(false);
      }
    });
    const approvalPromise = onAiToolApproval((event) => {
      if (disposed || event.sessionId !== runningSessionRef.current) return;
      // 等待用户确认属于正常进展，暂停“无输出”超时，确认完成后由后续输出重新计时。
      if (timeoutRef.current !== null) {
        window.clearTimeout(timeoutRef.current);
        timeoutRef.current = null;
      }
      setPendingApproval(event);
      setApprovalSubmitting(false);
      if (approvalExpiryTimeoutRef.current !== null) {
        window.clearTimeout(approvalExpiryTimeoutRef.current);
      }
      // 后端关闭事件是主路径；绝对到期时间用于系统休眠或事件丢失后的界面自愈。
      approvalExpiryTimeoutRef.current = window.setTimeout(
        () => {
          setPendingApproval((current) =>
            current?.approvalId === event.approvalId ? null : current
          );
          setApprovalSubmitting(false);
          if (runningSessionRef.current === event.sessionId) {
            setNotice('终端命令确认已超时，命令未执行。');
            timeoutRef.current = window.setTimeout(() => {
              if (runningSessionRef.current !== event.sessionId) return;
              timedOutSessionRef.current = event.sessionId;
              requestSessionStop(event.sessionId);
            }, AI_IDLE_TIMEOUT_MS);
          }
        },
        Math.max(0, event.expiresAtUnixMs - Date.now())
      );
    });
    const approvalClosedPromise = onAiToolApprovalClosed((event) => {
      if (disposed || event.sessionId !== runningSessionRef.current) return;
      if (approvalExpiryTimeoutRef.current !== null) {
        window.clearTimeout(approvalExpiryTimeoutRef.current);
        approvalExpiryTimeoutRef.current = null;
      }
      setPendingApproval((current) => (current?.approvalId === event.approvalId ? null : current));
      setApprovalSubmitting(false);
      if (event.resolution === 'timed_out') {
        setNotice('终端命令确认已超时，命令未执行。');
      }
      if (timeoutRef.current !== null) window.clearTimeout(timeoutRef.current);
      timeoutRef.current = window.setTimeout(() => {
        if (runningSessionRef.current !== event.sessionId) return;
        timedOutSessionRef.current = event.sessionId;
        requestSessionStop(event.sessionId);
      }, AI_IDLE_TIMEOUT_MS);
    });
    const subscriptions = [outputPromise, exitPromise, approvalPromise, approvalClosedPromise];
    void Promise.all(subscriptions)
      .then(() => {
        if (!disposed) setEventListenersReady(true);
      })
      .catch((error: unknown) => {
        if (!disposed) {
          setEventListenersReady(false);
          setNotice(
            error instanceof Error ? error.message : '初始化 AI 事件监听失败，请重启应用。'
          );
        }
      });
    void getAiProviderStatus()
      .then((statuses) => {
        if (disposed) return;
        setProviderAvailability(
          Object.fromEntries(statuses.map((item) => [item.id, item.available]))
        );
      })
      .catch((error: unknown) => {
        if (!disposed) {
          setNotice(error instanceof Error ? error.message : '读取 AI Provider 状态失败。');
        }
      });
    return () => {
      disposed = true;
      if (timeoutRef.current !== null) window.clearTimeout(timeoutRef.current);
      if (approvalExpiryTimeoutRef.current !== null) {
        window.clearTimeout(approvalExpiryTimeoutRef.current);
      }
      // 单个监听注册失败不能阻止其他已注册监听释放，清理阶段也不能产生未处理 Promise。
      subscriptions.forEach((subscription) => {
        void subscription.then((unlisten) => unlisten()).catch(() => undefined);
      });
    };
  }, [addMessage, requestSessionStop]);

  if (!visible) return null;

  /** 组装终端上下文并启动 Provider 会话，正常发送与失败重试共用这条路径。 */
  const sendQuestion = (
    question: string,
    selectedAttachment: AiAttachment | null,
    history = messages
  ) => {
    setNotice(null);
    setCanRetryQuestion(false);
    setStreamText('');
    setRunParts([]);
    runPartsRef.current = [];
    setLiveThinking('');
    setPendingApproval(null);
    if (approvalExpiryTimeoutRef.current !== null) {
      window.clearTimeout(approvalExpiryTimeoutRef.current);
      approvalExpiryTimeoutRef.current = null;
    }
    stderrLastRef.current = '';
    answerStreamedRef.current = false;
    if (!isDesktopRuntime()) {
      setNotice('浏览器预览不具备本机进程权限，请在 Nocterm 桌面客户端中使用 AI Provider。');
      return;
    }
    if (!eventListenersReady) {
      setNotice('AI 事件通道尚未就绪，请稍后重试；持续不可用时请重启应用。');
      return;
    }
    // 先绑定本轮 ID 再调用 IPC，避免极快退出的 Provider 事件早于 invoke 响应而被丢弃。
    const clientSessionId = `ai-${crypto.randomUUID()}`;
    runningSessionRef.current = clientSessionId;
    setRunningSessionId(clientSessionId);
    timeoutRef.current = window.setTimeout(() => {
      if (runningSessionRef.current !== clientSessionId) return;
      timedOutSessionRef.current = clientSessionId;
      requestSessionStop(clientSessionId);
    }, AI_IDLE_TIMEOUT_MS);
    const target = resolveAiTerminalTarget(activeSession, activeSessionStatus);
    const { context } = target;
    const turnPrompt = `${context}${buildAiPrompt(question, selectedAttachment)}`;
    void startAiSession(
      clientSessionId,
      conversationId,
      provider,
      `${context}${buildConversationContext(history)}${buildAiPrompt(question, selectedAttachment)}`,
      turnPrompt,
      undefined,
      target.connectionId,
      target.targetSessionId,
      commandPolicy
    )
      .then((result) => {
        if (result.sessionId !== clientSessionId && runningSessionRef.current === clientSessionId) {
          if (timeoutRef.current !== null) window.clearTimeout(timeoutRef.current);
          timeoutRef.current = null;
          runningSessionRef.current = null;
          setRunningSessionId(null);
          setNotice('AI Provider 返回了不匹配的会话标识。');
          setCanRetryQuestion(true);
        }
      })
      .catch((error: unknown) => {
        if (runningSessionRef.current !== clientSessionId) return;
        if (timeoutRef.current !== null) window.clearTimeout(timeoutRef.current);
        timeoutRef.current = null;
        runningSessionRef.current = null;
        setRunningSessionId(null);
        setNotice(error instanceof Error ? error.message : '启动 AI Provider 失败。');
        setCanRetryQuestion(true);
      });
  };

  // promptOverride 供建议卡片直接发送预置问题，绕过输入框草稿的异步时序。
  const submit = (event?: FormEvent, promptOverride?: string) => {
    event?.preventDefault();
    if (runningSessionRef.current) return;
    const content = (promptOverride ?? draft).trim();
    if (!content && !attachment) return;
    const selectedAttachment = attachment;
    const question = content || `请分析附件：${selectedAttachment?.name ?? '未命名文件'}`;
    const visibleMessage = selectedAttachment
      ? `${question}\n\n附件：${selectedAttachment.name}`
      : question;
    addMessage('user', visibleMessage);
    setDraft('');
    setAttachment(null);
    sendQuestion(question, selectedAttachment, messages);
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
    // 附件正文不落盘、无法随重试重读，只发送文本部分并去掉“附件：名称”标记。
    const question = retried.content.replace(/\n\n附件：[^\n]*$/, '');
    messages.slice(lastUserIndex).forEach((message) => deleteMessage(message.id));
    addMessage('user', question);
    sendQuestion(question, null, messages.slice(0, lastUserIndex));
  };

  const handleComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault();
      submit();
    }
  };

  const resolveApproval = (approved: boolean) => {
    if (!pendingApproval || approvalSubmitting) return;
    const approvalId = pendingApproval.approvalId;
    const sessionId = pendingApproval.sessionId;
    setApprovalSubmitting(true);
    void resolveAiToolApproval(approvalId, sessionId, approved)
      .then(() => {
        if (approvalExpiryTimeoutRef.current !== null) {
          window.clearTimeout(approvalExpiryTimeoutRef.current);
          approvalExpiryTimeoutRef.current = null;
        }
        setPendingApproval((current) => (current?.approvalId === approvalId ? null : current));
        setApprovalSubmitting(false);
        if (runningSessionRef.current) {
          if (timeoutRef.current !== null) window.clearTimeout(timeoutRef.current);
          timeoutRef.current = window.setTimeout(() => {
            const sessionId = runningSessionRef.current;
            if (!sessionId) return;
            timedOutSessionRef.current = sessionId;
            requestSessionStop(sessionId);
          }, AI_IDLE_TIMEOUT_MS);
        }
      })
      .catch((error: unknown) => {
        setApprovalSubmitting(false);
        setNotice(error instanceof Error ? error.message : '提交命令确认失败。');
      });
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
    setNotice(null);
    setCanRetryQuestion(false);
    setRunParts([]);
    runPartsRef.current = [];
    setLiveThinking('');
    composerRef.current?.focus();
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
        <div className={styles.historyPanel}>
          <div className={styles.historyHeader}>
            <span>历史会话</span>
            <div className={styles.historyActions}>
              {messages.length > 0 ? (
                <button
                  disabled={Boolean(runningSessionId)}
                  onClick={async () => {
                    if (!(await resetProviderConversation(conversationId))) return;
                    clearMessages();
                  }}
                  type="button"
                >
                  清空
                </button>
              ) : null}
            </div>
          </div>
          <div className={styles.historyList}>
            {conversations
              .slice()
              .sort((a, b) => b.updatedAt - a.updatedAt)
              .map((conversation) => (
                <div
                  className={`${styles.historyItem} ${conversation.id === activeConversationId ? styles.historyItemActive : ''}`}
                  key={conversation.id}
                >
                  <button
                    className={styles.historySelect}
                    disabled={Boolean(runningSessionId)}
                    onClick={async () => {
                      if (conversation.id !== conversationId) {
                        if (!(await resetProviderConversation(conversationId))) return;
                      }
                      selectConversation(conversation.id);
                      setAttachment(null);
                      setHistoryOpen(false);
                    }}
                    type="button"
                  >
                    <span>{conversation.title}</span>
                  </button>
                  <button
                    aria-label={`删除会话：${conversation.title}`}
                    className={styles.historyDelete}
                    disabled={Boolean(runningSessionId)}
                    onClick={async () => {
                      if (!(await resetProviderConversation(conversation.id))) return;
                      deleteConversation(conversation.id);
                      setAttachment(null);
                    }}
                    title="删除会话"
                    type="button"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13M10 11v5M14 11v5" />
                    </svg>
                  </button>
                </div>
              ))}
          </div>
        </div>
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
          liveThinking={liveThinking}
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

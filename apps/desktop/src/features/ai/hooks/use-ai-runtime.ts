import { useCallback, useEffect, useRef, useState } from 'react';

import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import {
  getAiProviderStatus,
  onAiExit,
  onAiOutput,
  onAiToolApproval,
  onAiToolApprovalClosed,
  resolveAiToolApproval,
  startAiSession,
  stopAiSession,
} from '../api/ai-client';
import { buildAiPrompt, type AiAttachment } from '../model/ai-attachment';
import { buildConversationContext } from '../model/ai-conversation';
import {
  canStartAiRuntimeTask,
  completeAiRuntimeTask,
  recordCompletedAiTaskResult,
  removeAiRuntimeTask,
  removeCompletedAiTaskResult,
  updateAiRuntimeTask,
  type AiCompletedTaskResults,
  type AiRuntimeTask,
  type AiRuntimeTasks,
  type AiTaskOutcome,
} from '../model/ai-runtime-task';
import type { AiTerminalTarget } from '../model/ai-target';
import { aiRuntimeErrorMessage, reduceAiRuntimeOutput } from '../model/ai-runtime-output';
import type { AiCommandPolicy, AiMessage, AiMessagePart, AiProviderId } from '../model/ai-types';

// Provider 握手或网络请求异常时，不能让任何一个任务无限显示“输出中”。
const AI_IDLE_TIMEOUT_MS = 90_000;
const AI_IDLE_TIMEOUT_SECONDS = AI_IDLE_TIMEOUT_MS / 1_000;

interface AiRuntimeOptions {
  addMessage: (
    conversationId: string,
    role: AiMessage['role'],
    content: string,
    parts?: AiMessagePart[]
  ) => void;
}

interface StartAiQuestion {
  question: string;
  attachments: readonly AiAttachment[];
  history: AiMessage[];
  conversationId: string;
  provider: AiProviderId;
  target: AiTerminalTarget;
  commandPolicy: AiCommandPolicy;
}

export interface AiRuntimePresentation {
  notice: string | null;
  canRetryQuestion: boolean;
  /** 失败重试所需的瞬时上下文；成功后随 presentation 清理，不进入消息持久化。 */
  retryAttachments?: readonly AiAttachment[];
}

interface RuntimeTaskRef {
  task: AiRuntimeTask;
  attachments: readonly AiAttachment[];
  answerStreamed: boolean;
  stderrLast: string;
  timedOut: boolean;
  idleTimeoutId: number | null;
  approvalExpiryTimeoutId: number | null;
}

const runtimeTaskSnapshots = (taskRefs: Map<string, RuntimeTaskRef>): AiRuntimeTasks =>
  Object.fromEntries(Array.from(taskRefs, ([sessionId, runtime]) => [sessionId, runtime.task]));

/**
 * Provider 运行态按 sessionId 隔离；切换 Tab 只改变展示，不改变事件归属。
 * 对话消息仍由 Store 持久化，这里只管理进程存活期间的流、审批与计时器。
 */
export function useAiRuntime({ addMessage }: AiRuntimeOptions) {
  const [tasks, setTasks] = useState<AiRuntimeTasks>({});
  const [completedTaskResults, setCompletedTaskResults] = useState<AiCompletedTaskResults>({});
  const [presentations, setPresentations] = useState<
    Record<string, AiRuntimePresentation | undefined>
  >({});
  const [globalNotice, setGlobalNotice] = useState<string | null>(null);
  const [providerAvailability, setProviderAvailability] = useState<Record<string, boolean>>({});
  const [eventListenersReady, setEventListenersReady] = useState(false);
  // ref 是事件回调的权威快照，避免并发输出早于 React state 提交而丢失。
  const taskRefs = useRef(new Map<string, RuntimeTaskRef>());

  const setConversationPresentation = useCallback(
    (conversationId: string, presentation: AiRuntimePresentation | null) => {
      setPresentations((current) => {
        if (presentation) return { ...current, [conversationId]: presentation };
        if (!current[conversationId]) return current;
        const next = { ...current };
        delete next[conversationId];
        return next;
      });
    },
    []
  );

  const recordTaskResult = useCallback((task: AiRuntimeTask, outcome: AiTaskOutcome) => {
    const result = completeAiRuntimeTask(task, outcome, Date.now());
    setCompletedTaskResults((current) => recordCompletedAiTaskResult(current, result));
  }, []);

  const markTaskResultRead = useCallback((conversationId: string) => {
    setCompletedTaskResults((current) => removeCompletedAiTaskResult(current, conversationId));
  }, []);

  const markAllTaskResultsRead = useCallback(() => {
    setCompletedTaskResults({});
  }, []);

  const updateTask = useCallback(
    (sessionId: string, updater: (task: AiRuntimeTask) => AiRuntimeTask) => {
      const runtime = taskRefs.current.get(sessionId);
      if (!runtime) return;
      runtime.task = updater(runtime.task);
      setTasks((current) => updateAiRuntimeTask(current, sessionId, runtime.task));
    },
    []
  );

  /** 清理单个任务的资源，绝不能影响其他终端的计时器。 */
  const clearTaskTimers = useCallback((runtime: RuntimeTaskRef) => {
    if (runtime.idleTimeoutId !== null) window.clearTimeout(runtime.idleTimeoutId);
    if (runtime.approvalExpiryTimeoutId !== null) {
      window.clearTimeout(runtime.approvalExpiryTimeoutId);
    }
    runtime.idleTimeoutId = null;
    runtime.approvalExpiryTimeoutId = null;
  }, []);

  const removeTask = useCallback(
    (sessionId: string): RuntimeTaskRef | null => {
      const runtime = taskRefs.current.get(sessionId);
      if (!runtime) return null;
      clearTaskTimers(runtime);
      taskRefs.current.delete(sessionId);
      setTasks((current) => removeAiRuntimeTask(current, sessionId));
      return runtime;
    },
    [clearTaskTimers]
  );

  // 停止只发送请求，最终回答和错误状态仍统一由对应 sessionId 的退出事件收敛。
  const requestSessionStop = useCallback(
    (sessionId: string) => {
      if (!taskRefs.current.has(sessionId)) return;
      void stopAiSession(sessionId).catch((error: unknown) => {
        const runtime = taskRefs.current.get(sessionId);
        if (!runtime) return;
        runtime.timedOut = false;
        updateTask(sessionId, (task) => ({
          ...task,
          notice: aiRuntimeErrorMessage(error, '停止 AI 会话失败。'),
        }));
      });
    },
    [updateTask]
  );

  const restartIdleTimeout = useCallback(
    (sessionId: string) => {
      const runtime = taskRefs.current.get(sessionId);
      if (!runtime) return;
      if (runtime.idleTimeoutId !== null) window.clearTimeout(runtime.idleTimeoutId);
      runtime.idleTimeoutId = window.setTimeout(() => {
        const current = taskRefs.current.get(sessionId);
        if (!current) return;
        current.timedOut = true;
        requestSessionStop(sessionId);
      }, AI_IDLE_TIMEOUT_MS);
    },
    [requestSessionStop]
  );

  const failTaskStart = useCallback(
    (sessionId: string, message: string) => {
      const runtime = removeTask(sessionId);
      if (!runtime) return;
      setConversationPresentation(runtime.task.conversationId, {
        notice: message,
        canRetryQuestion: true,
        retryAttachments: runtime.attachments,
      });
      recordTaskResult(runtime.task, 'failed');
    },
    [recordTaskResult, removeTask, setConversationPresentation]
  );

  useEffect(() => {
    // 四类事件监听仅注册一次，每个回调按事件携带的 sessionId 精确路由。
    if (!isDesktopRuntime()) return;
    let disposed = false;
    const runtimeTasks = taskRefs.current;
    const outputPromise = onAiOutput((event) => {
      if (disposed) return;
      const runtime = taskRefs.current.get(event.sessionId);
      if (!runtime) return;
      restartIdleTimeout(event.sessionId);
      if (event.stream === 'stderr') {
        const line = event.data.trim();
        if (line) runtime.stderrLast = line;
        return;
      }
      const reduced = reduceAiRuntimeOutput(event.data, {
        text: runtime.task.streamText,
        parts: runtime.task.runParts,
        answerStreamed: runtime.answerStreamed,
      });
      if (!reduced.changed) return;
      runtime.answerStreamed = reduced.answerStreamed;
      updateTask(event.sessionId, (task) => ({
        ...task,
        streamText: reduced.text,
        runParts: reduced.parts,
      }));
    });
    const exitPromise = onAiExit((event) => {
      if (disposed) return;
      const runtime = removeTask(event.sessionId);
      if (!runtime) return;
      const { task } = runtime;
      if (task.streamText.trim()) {
        addMessage(task.conversationId, 'assistant', task.streamText.trim(), task.runParts);
      }
      let outcome: AiTaskOutcome;
      if (runtime.timedOut) {
        outcome = 'timed_out';
        setConversationPresentation(task.conversationId, {
          notice: `AI 会话超过 ${AI_IDLE_TIMEOUT_SECONDS} 秒没有新的输出，已自动停止。请检查 AI 服务或终端连接状态后重试。`,
          canRetryQuestion: true,
          retryAttachments: runtime.attachments,
        });
      } else if (event.cancelled) {
        outcome = 'stopped';
        setConversationPresentation(task.conversationId, {
          notice: 'AI 会话已停止。',
          canRetryQuestion: true,
          retryAttachments: runtime.attachments,
        });
      } else if (event.code !== 0) {
        outcome = 'failed';
        const detail = runtime.stderrLast.slice(0, 200);
        setConversationPresentation(task.conversationId, {
          notice: `AI Provider 异常退出（代码 ${event.code ?? '未知'}）${detail ? `：${detail}` : ''}。`,
          canRetryQuestion: true,
          retryAttachments: runtime.attachments,
        });
      } else {
        outcome = 'completed';
        setConversationPresentation(task.conversationId, null);
      }
      recordTaskResult(task, outcome);
    });
    const approvalPromise = onAiToolApproval((event) => {
      if (disposed) return;
      const runtime = taskRefs.current.get(event.sessionId);
      if (!runtime) return;
      // 等待人工确认不算 Provider 空闲；到期后只恢复该任务的空闲计时。
      if (runtime.idleTimeoutId !== null) window.clearTimeout(runtime.idleTimeoutId);
      runtime.idleTimeoutId = null;
      if (runtime.approvalExpiryTimeoutId !== null) {
        window.clearTimeout(runtime.approvalExpiryTimeoutId);
      }
      updateTask(event.sessionId, (task) => ({
        ...task,
        pendingApproval: event,
        approvalSubmitting: false,
        notice: null,
      }));
      runtime.approvalExpiryTimeoutId = window.setTimeout(
        () => {
          const current = taskRefs.current.get(event.sessionId);
          if (current?.task.pendingApproval?.approvalId !== event.approvalId) return;
          current.approvalExpiryTimeoutId = null;
          updateTask(event.sessionId, (task) => ({
            ...task,
            pendingApproval: null,
            approvalSubmitting: false,
            notice: '终端命令确认已超时，命令未执行。',
          }));
          restartIdleTimeout(event.sessionId);
        },
        Math.max(0, event.expiresAtUnixMs - Date.now())
      );
    });
    const approvalClosedPromise = onAiToolApprovalClosed((event) => {
      if (disposed) return;
      const runtime = taskRefs.current.get(event.sessionId);
      if (!runtime) return;
      if (runtime.approvalExpiryTimeoutId !== null) {
        window.clearTimeout(runtime.approvalExpiryTimeoutId);
        runtime.approvalExpiryTimeoutId = null;
      }
      updateTask(event.sessionId, (task) => ({
        ...task,
        pendingApproval:
          task.pendingApproval?.approvalId === event.approvalId ? null : task.pendingApproval,
        approvalSubmitting: false,
        notice: event.resolution === 'timed_out' ? '终端命令确认已超时，命令未执行。' : task.notice,
      }));
      restartIdleTimeout(event.sessionId);
    });
    const subscriptions = [outputPromise, exitPromise, approvalPromise, approvalClosedPromise];
    void Promise.all(subscriptions)
      .then(() => {
        if (!disposed) setEventListenersReady(true);
      })
      .catch((error: unknown) => {
        if (!disposed) {
          setEventListenersReady(false);
          setGlobalNotice(aiRuntimeErrorMessage(error, '初始化 AI 事件监听失败，请重启应用。'));
        }
      });
    void getAiProviderStatus()
      .then((statuses) => {
        if (!disposed) {
          setProviderAvailability(
            Object.fromEntries(statuses.map((item) => [item.id, item.available]))
          );
        }
      })
      .catch((error: unknown) => {
        if (!disposed) {
          setGlobalNotice(aiRuntimeErrorMessage(error, '读取 AI Provider 状态失败。'));
        }
      });
    return () => {
      disposed = true;
      runtimeTasks.forEach(clearTaskTimers);
      subscriptions.forEach((subscription) => {
        // 某个监听注册失败不能阻止其他已注册监听释放。
        void subscription.then((unlisten) => unlisten()).catch(() => undefined);
      });
    };
  }, [
    addMessage,
    clearTaskTimers,
    removeTask,
    recordTaskResult,
    restartIdleTimeout,
    setConversationPresentation,
    updateTask,
  ]);

  /** 先注册客户端 sessionId，再调用后端，保证快速输出也能找到所属任务。 */
  const sendQuestion = useCallback(
    ({
      question,
      attachments,
      history,
      conversationId,
      provider,
      target,
      commandPolicy,
    }: StartAiQuestion) => {
      setConversationPresentation(conversationId, null);
      markTaskResultRead(conversationId);
      if (!target.binding || !target.workspaceSessionId) {
        setConversationPresentation(conversationId, {
          notice: '当前终端不可用，无法启动 AI 任务。',
          canRetryQuestion: true,
          retryAttachments: attachments,
        });
        return;
      }
      if (!isDesktopRuntime()) {
        setConversationPresentation(conversationId, {
          notice: '浏览器预览不具备本机进程权限，请在 Nocterm 桌面客户端中使用 AI Provider。',
          canRetryQuestion: true,
          retryAttachments: attachments,
        });
        return;
      }
      if (!eventListenersReady) {
        setConversationPresentation(conversationId, {
          notice: 'AI 事件通道尚未就绪，请稍后重试；持续不可用时请重启应用。',
          canRetryQuestion: true,
          retryAttachments: attachments,
        });
        return;
      }
      const currentTasks = runtimeTaskSnapshots(taskRefs.current);
      if (!canStartAiRuntimeTask(currentTasks, conversationId, target.binding)) {
        setConversationPresentation(conversationId, {
          notice: '当前终端已有 AI 任务正在运行，请先处理或停止该任务。',
          canRetryQuestion: false,
        });
        return;
      }

      const clientSessionId = `ai-${crypto.randomUUID()}`;
      const task: AiRuntimeTask = {
        sessionId: clientSessionId,
        conversationId,
        target: target.binding,
        sourceTerminalSessionId: target.workspaceSessionId,
        provider,
        startedAt: Date.now(),
        streamText: '',
        runParts: [],
        pendingApproval: null,
        approvalSubmitting: false,
        notice: null,
      };
      taskRefs.current.set(clientSessionId, {
        task,
        attachments: [...attachments],
        answerStreamed: false,
        stderrLast: '',
        timedOut: false,
        idleTimeoutId: null,
        approvalExpiryTimeoutId: null,
      });
      setTasks((current) => ({ ...current, [clientSessionId]: task }));
      restartIdleTimeout(clientSessionId);

      const prompt = `${target.context}${buildAiPrompt(question, attachments)}`;
      void startAiSession(
        clientSessionId,
        conversationId,
        provider,
        `${target.context}${buildConversationContext(history)}${buildAiPrompt(question, attachments)}`,
        prompt,
        undefined,
        target.connectionId,
        target.targetSessionId,
        commandPolicy
      )
        .then((result) => {
          if (result.sessionId !== clientSessionId && taskRefs.current.has(clientSessionId)) {
            failTaskStart(clientSessionId, 'AI Provider 返回了不匹配的会话标识。');
          }
        })
        .catch((error: unknown) => {
          if (!taskRefs.current.has(clientSessionId)) return;
          failTaskStart(clientSessionId, aiRuntimeErrorMessage(error, '启动 AI Provider 失败。'));
        });
    },
    [
      eventListenersReady,
      failTaskStart,
      markTaskResultRead,
      restartIdleTimeout,
      setConversationPresentation,
    ]
  );

  /** 审批动作显式携带 sessionId，查看后台会话时也不会提交到当前终端。 */
  const resolveApproval = useCallback(
    (sessionId: string, approved: boolean) => {
      const runtime = taskRefs.current.get(sessionId);
      const approval = runtime?.task.pendingApproval;
      if (!runtime || !approval || runtime.task.approvalSubmitting) return;
      updateTask(sessionId, (task) => ({ ...task, approvalSubmitting: true }));
      void resolveAiToolApproval(approval.approvalId, sessionId, approved)
        .then(() => {
          const current = taskRefs.current.get(sessionId);
          if (!current) return;
          if (current.approvalExpiryTimeoutId !== null) {
            window.clearTimeout(current.approvalExpiryTimeoutId);
            current.approvalExpiryTimeoutId = null;
          }
          updateTask(sessionId, (task) => ({
            ...task,
            pendingApproval:
              task.pendingApproval?.approvalId === approval.approvalId
                ? null
                : task.pendingApproval,
            approvalSubmitting: false,
          }));
          restartIdleTimeout(sessionId);
        })
        .catch((error: unknown) => {
          updateTask(sessionId, (task) => ({
            ...task,
            approvalSubmitting: false,
            notice: aiRuntimeErrorMessage(error, '提交命令确认失败。'),
          }));
        });
    },
    [restartIdleTimeout, updateTask]
  );

  const clearConversationPresentation = useCallback(
    (conversationId: string | null) => {
      if (conversationId) setConversationPresentation(conversationId, null);
    },
    [setConversationPresentation]
  );

  return {
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
  };
}

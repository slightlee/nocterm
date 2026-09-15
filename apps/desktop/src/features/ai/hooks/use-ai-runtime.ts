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
import type { AiTerminalTarget } from '../model/ai-target';
import { reduceAiRuntimeOutput } from '../model/ai-runtime-output';
import type {
  AiCommandPolicy,
  AiMessage,
  AiMessagePart,
  AiProviderId,
  AiToolApprovalEvent,
} from '../model/ai-types';

// Provider 握手或网络请求异常时，不能让面板无限显示“输出中”。
const AI_IDLE_TIMEOUT_MS = 90_000;
const AI_IDLE_TIMEOUT_SECONDS = AI_IDLE_TIMEOUT_MS / 1_000;

interface AiRuntimeOptions {
  addMessage: (role: AiMessage['role'], content: string, parts?: AiMessagePart[]) => void;
}

interface StartAiQuestion {
  question: string;
  attachment: AiAttachment | null;
  history: AiMessage[];
  conversationId: string;
  provider: AiProviderId;
  target: AiTerminalTarget;
  commandPolicy: AiCommandPolicy;
}

/**
 * 管理 Provider 进程的事件流、超时、审批和停止；面板关闭或切页时仍保持挂载。
 * 这里不保存会话消息，避免运行时生命周期和 Zustand 对话模型相互耦合。
 *
 * 生命周期约束：
 * - 客户端生成会话 ID，并在启动 IPC 前绑定，确保不会漏掉快速返回的事件；
 * - Provider 退出事件负责持久化最终回答，停止请求本身不提前结束前端状态；
 * - 每个空闲计时器和审批计时器都由本 Hook 创建、替换并在卸载时释放；
 * - 所有事件都校验活动会话 ID，迟到事件不能污染下一轮对话；
 * - React state 与 ref 必须同步清理，兼顾渲染一致性和事件回调的即时读取。
 */
export function useAiRuntime({ addMessage }: AiRuntimeOptions) {
  /**
   * React state 驱动展示，对应 ref 保存事件回调必须立即读取的最新快照。
   * Provider 事件不受 React 提交时机约束，因此不能只依赖闭包中的 state。
   *
   * refs 分为三组：
   * - 当前运行标识，用于丢弃其他会话或迟到的事件；
   * - 流式内容快照，用于在 React 提交前连续归并输出；
   * - 定时器与失败诊断，用于统一处理超时、审批过期和异常退出。
   */
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
  const runningSessionRef = useRef<string | null>(null);
  const streamTextRef = useRef('');
  const runPartsRef = useRef<AiMessagePart[]>([]);
  const stderrLastRef = useRef('');
  const answerStreamedRef = useRef(false);
  const timeoutRef = useRef<number | null>(null);
  const approvalExpiryTimeoutRef = useRef<number | null>(null);
  const timedOutSessionRef = useRef<string | null>(null);

  /** 同步清理当前轮次的流式快照，防止下一轮的首个事件拼接到旧 ref。 */
  const clearRunPresentation = useCallback(() => {
    streamTextRef.current = '';
    runPartsRef.current = [];
    answerStreamedRef.current = false;
    setStreamText('');
    setRunParts([]);
    setLiveThinking('');
    setPendingApproval(null);
    setApprovalSubmitting(false);
  }, []);

  // stopAiSession 只发出停止请求，最终状态仍由退出事件统一收敛。
  // 请求失败时保持 runningSessionId，避免前端误判 Provider 已经退出。
  const requestSessionStop = useCallback((sessionId: string) => {
    void stopAiSession(sessionId).catch((error: unknown) => {
      if (runningSessionRef.current !== sessionId) return;
      timedOutSessionRef.current = null;
      setNotice(error instanceof Error ? error.message : '停止 AI 会话失败。');
      setCanRetryQuestion(false);
    });
  }, []);

  useEffect(() => {
    // 事件监听只注册一次，并覆盖输出、退出和审批这三类运行时状态转换。
    // AiPanel 隐藏时 Hook 仍然挂载，因此折叠面板不会中断正在执行的任务。
    if (!isDesktopRuntime()) return;
    let disposed = false;
    const outputPromise = onAiOutput((event) => {
      if (disposed || event.sessionId !== runningSessionRef.current) return;
      // 任意 Provider 输出都代表任务仍有进展，重新计算空闲超时。
      if (timeoutRef.current !== null) window.clearTimeout(timeoutRef.current);
      timeoutRef.current = window.setTimeout(() => {
        if (runningSessionRef.current !== event.sessionId) return;
        timedOutSessionRef.current = event.sessionId;
        requestSessionStop(event.sessionId);
      }, AI_IDLE_TIMEOUT_MS);
      if (event.stream === 'stderr') {
        // stderr 仅作为失败诊断，不能混入最终助手消息。
        const line = event.data.trim();
        if (line) stderrLastRef.current = line;
        return;
      }
      const reduced = reduceAiRuntimeOutput(event.data, {
        text: streamTextRef.current,
        parts: runPartsRef.current,
        answerStreamed: answerStreamedRef.current,
      });
      if (reduced.thinkingDelta) {
        setLiveThinking((current) => (current + reduced.thinkingDelta).slice(-160));
      }
      if (reduced.clearThinking) setLiveThinking('');
      if (!reduced.changed) return;
      // 先更新 ref，再提交 React state；Provider 可能在下次渲染前立即退出。
      streamTextRef.current = reduced.text;
      runPartsRef.current = reduced.parts;
      answerStreamedRef.current = reduced.answerStreamed;
      setStreamText(reduced.text);
      setRunParts(reduced.parts);
    });
    const exitPromise = onAiExit((event) => {
      if (disposed || event.sessionId !== runningSessionRef.current) return;
      // 退出事件是唯一完成路径：停止计时、持久化结果并释放审批状态。
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
      clearRunPresentation();
      if (timedOutSessionRef.current === event.sessionId) {
        timedOutSessionRef.current = null;
        setNotice(
          `AI 会话超过 ${AI_IDLE_TIMEOUT_SECONDS} 秒没有新的输出，已自动停止。请检查 AI 服务或终端连接状态后重试。`
        );
        setCanRetryQuestion(true);
      } else if (event.cancelled) {
        setNotice('AI 会话已停止。');
        setCanRetryQuestion(true);
      } else if (event.code !== 0) {
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
      // 等待人工确认不是 Provider 卡死，审批期间暂停空闲超时。
      if (timeoutRef.current !== null) {
        window.clearTimeout(timeoutRef.current);
        timeoutRef.current = null;
      }
      setPendingApproval(event);
      setApprovalSubmitting(false);
      if (approvalExpiryTimeoutRef.current !== null) {
        window.clearTimeout(approvalExpiryTimeoutRef.current);
      }
      approvalExpiryTimeoutRef.current = window.setTimeout(
        () => {
          // 后端关闭事件可能因系统休眠而延迟，本地绝对到期时间用于界面自愈。
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
      // 后端是审批结果的权威来源；关闭后恢复 Provider 空闲计时。
      if (approvalExpiryTimeoutRef.current !== null) {
        window.clearTimeout(approvalExpiryTimeoutRef.current);
        approvalExpiryTimeoutRef.current = null;
      }
      setPendingApproval((current) => (current?.approvalId === event.approvalId ? null : current));
      setApprovalSubmitting(false);
      if (event.resolution === 'timed_out') setNotice('终端命令确认已超时，命令未执行。');
      if (timeoutRef.current !== null) window.clearTimeout(timeoutRef.current);
      timeoutRef.current = window.setTimeout(() => {
        if (runningSessionRef.current !== event.sessionId) return;
        timedOutSessionRef.current = event.sessionId;
        requestSessionStop(event.sessionId);
      }, AI_IDLE_TIMEOUT_MS);
    });
    const subscriptions = [outputPromise, exitPromise, approvalPromise, approvalClosedPromise];
    // 必须等待四个监听全部注册成功，避免会话启动后缺失任一关键事件。
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
        if (!disposed) {
          setProviderAvailability(
            Object.fromEntries(statuses.map((item) => [item.id, item.available]))
          );
        }
      })
      .catch((error: unknown) => {
        if (!disposed)
          setNotice(error instanceof Error ? error.message : '读取 AI Provider 状态失败。');
      });
    return () => {
      disposed = true;
      if (timeoutRef.current !== null) window.clearTimeout(timeoutRef.current);
      if (approvalExpiryTimeoutRef.current !== null) {
        window.clearTimeout(approvalExpiryTimeoutRef.current);
      }
      subscriptions.forEach((subscription) => {
        // 某个监听注册失败不能阻止其他已注册监听释放。
        void subscription.then((unlisten) => unlisten()).catch(() => undefined);
      });
    };
  }, [addMessage, clearRunPresentation, requestSessionStop]);

  /** 组装终端上下文并启动 Provider，会话历史与当前轮次分别传给适配层。 */
  const sendQuestion = useCallback(
    ({
      question,
      attachment,
      history,
      conversationId,
      provider,
      target,
      commandPolicy,
    }: StartAiQuestion) => {
      setNotice(null);
      setCanRetryQuestion(false);
      clearRunPresentation();
      if (approvalExpiryTimeoutRef.current !== null) {
        window.clearTimeout(approvalExpiryTimeoutRef.current);
        approvalExpiryTimeoutRef.current = null;
      }
      stderrLastRef.current = '';
      timedOutSessionRef.current = null;
      if (!isDesktopRuntime()) {
        setNotice('浏览器预览不具备本机进程权限，请在 Nocterm 桌面客户端中使用 AI Provider。');
        return;
      }
      if (!eventListenersReady) {
        setNotice('AI 事件通道尚未就绪，请稍后重试；持续不可用时请重启应用。');
        return;
      }
      // 先绑定客户端会话 ID 再 invoke，避免极快退出的 Provider 事件被过滤。
      const clientSessionId = `ai-${crypto.randomUUID()}`;
      runningSessionRef.current = clientSessionId;
      setRunningSessionId(clientSessionId);
      timeoutRef.current = window.setTimeout(() => {
        if (runningSessionRef.current !== clientSessionId) return;
        timedOutSessionRef.current = clientSessionId;
        requestSessionStop(clientSessionId);
      }, AI_IDLE_TIMEOUT_MS);
      const prompt = `${target.context}${buildAiPrompt(question, attachment)}`;
      void startAiSession(
        clientSessionId,
        conversationId,
        provider,
        `${target.context}${buildConversationContext(history)}${buildAiPrompt(question, attachment)}`,
        prompt,
        undefined,
        target.connectionId,
        target.targetSessionId,
        commandPolicy
      )
        .then((result) => {
          if (
            result.sessionId !== clientSessionId &&
            runningSessionRef.current === clientSessionId
          ) {
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
    },
    [clearRunPresentation, eventListenersReady, requestSessionStop]
  );

  /** 提交审批后等待后端关闭事件；IPC 成功不等于命令已经完成。 */
  const resolveApproval = useCallback(
    (approved: boolean) => {
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
              const currentSessionId = runningSessionRef.current;
              if (!currentSessionId) return;
              timedOutSessionRef.current = currentSessionId;
              requestSessionStop(currentSessionId);
            }, AI_IDLE_TIMEOUT_MS);
          }
        })
        .catch((error: unknown) => {
          setApprovalSubmitting(false);
          setNotice(error instanceof Error ? error.message : '提交命令确认失败。');
        });
    },
    [approvalSubmitting, pendingApproval, requestSessionStop]
  );

  /** 新建会话时清理上一轮的瞬时展示状态；运行中的任务不调用此入口。 */
  const resetPresentation = useCallback(() => {
    setNotice(null);
    setCanRetryQuestion(false);
    clearRunPresentation();
    stderrLastRef.current = '';
    timedOutSessionRef.current = null;
  }, [clearRunPresentation]);

  return {
    notice,
    setNotice,
    canRetryQuestion,
    runningSessionId,
    streamText,
    runParts,
    liveThinking,
    pendingApproval,
    approvalSubmitting,
    providerAvailability,
    sendQuestion,
    resolveApproval,
    requestSessionStop,
    resetPresentation,
  };
}

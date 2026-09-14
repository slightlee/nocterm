import { Fragment } from 'react';

import type { AiMessage, AiMessagePart, AiToolApprovalEvent } from '../model/ai-types';
import { AiMarkdown } from './AiMarkdown';
import styles from './AiPanel.module.css';

const SUGGESTIONS = [
  {
    title: '解释终端报错',
    description: '分析错误原因并提供解决方案',
    prompt: '解释当前终端里的错误，并给出处理建议',
    icon: 'spark',
  },
  {
    title: '检查部署状态',
    description: '检查服务与容器运行状态',
    prompt: '检查当前服务器的部署状态',
    icon: 'server',
  },
  {
    title: '分析日志',
    description: '快速提取关键错误与告警',
    prompt: '分析当前终端中的运行日志，找出关键错误和告警',
    icon: 'document',
  },
  {
    title: '生成运维命令',
    description: '生成安全的诊断或修复命令',
    prompt: '根据当前问题生成安全、可审阅的运维命令',
    icon: 'command',
  },
] as const;

interface AiConversationViewProps {
  messages: AiMessage[];
  providerName: string;
  running: boolean;
  streamText: string;
  runParts: AiMessagePart[];
  liveThinking: string;
  pendingApproval: AiToolApprovalEvent | null;
  approvalSubmitting: boolean;
  notice: string | null;
  lastUserMessageId?: string;
  onSuggestion: (prompt: string) => void;
  onRetry: () => void;
  onResolveApproval: (approved: boolean) => void;
}

/** 中性终端助手头像统一用于历史回复与流式输出，避免用装饰性星星表达身份。 */
function AssistantAvatar() {
  return (
    <span className={styles.assistantAvatar} aria-hidden="true">
      <svg viewBox="0 0 24 24">
        <path d="M9 4h6M12 4V2M6 7h12a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2Z" />
        <path d="M8 12h1M15 12h1M9 16h6" />
      </svg>
    </span>
  );
}

/** 已完成与流式消息共用同一渲染器，工具活动不会在任务结束时改变位置或消失。 */
function AssistantContent({ content, parts }: { content: string; parts?: AiMessagePart[] }) {
  if (!parts?.length) return <AiMarkdown content={content} />;
  return parts.map((part, index) =>
    part.type === 'text' ? (
      <AiMarkdown content={part.content} key={`text-${index}`} />
    ) : (
      <div className={styles.activityLine} key={`activity-${index}-${part.content}`}>
        <span className={styles.activityKind}>{part.kind === 'tool' ? '执行' : '思考'}</span>
        <span>{part.content}</span>
      </div>
    )
  );
}

/** 纯展示会话区域；Provider 生命周期与持久化仍由 AiPanel 统一编排。 */
export function AiConversationView({
  messages,
  providerName,
  running,
  streamText,
  runParts,
  liveThinking,
  pendingApproval,
  approvalSubmitting,
  notice,
  lastUserMessageId,
  onSuggestion,
  onRetry,
  onResolveApproval,
}: AiConversationViewProps) {
  if (messages.length === 0) {
    return (
      <div className={styles.emptyState}>
        <h3>让助手协助你处理当前终端任务</h3>
        <div className={styles.taskGrid}>
          {SUGGESTIONS.map((suggestion) => (
            <button
              className={styles.taskCard}
              key={suggestion.title}
              onClick={() => onSuggestion(suggestion.prompt)}
              type="button"
            >
              <span className={styles.taskIcon} aria-hidden="true">
                <svg viewBox="0 0 24 24">
                  {suggestion.icon === 'spark' ? (
                    <path d="m12 3 1.6 5.4L19 10l-5.4 1.6L12 17l-1.6-5.4L5 10l5.4-1.6L12 3ZM19 16l.7 2.3L22 19l-2.3.7L19 22l-.7-2.3L16 19l2.3-.7L19 16Z" />
                  ) : suggestion.icon === 'server' ? (
                    <path d="M4 5h16v6H4zM4 13h16v6H4zM7 8h.01M7 16h.01M11 8h6M11 16h6" />
                  ) : suggestion.icon === 'document' ? (
                    <path d="M6 3h9l3 3v15H6zM9 11h6M9 15h6M9 7h3" />
                  ) : (
                    <path d="M5 4h14v16H5zM8 8h8M8 12h5M8 16h8" />
                  )}
                </svg>
              </span>
              <span className={styles.taskCopy}>
                <strong>{suggestion.title}</strong>
                <small>{suggestion.description}</small>
              </span>
            </button>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className={styles.messageList}>
      {messages.map((message) => (
        <Fragment key={message.id}>
          <article
            className={`${styles.message} ${message.role === 'user' ? styles.userMessage : styles.assistantMessage}`}
          >
            {message.role === 'assistant' ? <AssistantAvatar /> : null}
            <div className={styles.messageBody}>
              <span className={styles.messageRole}>
                {message.role === 'user' ? '你' : providerName}
              </span>
              {message.role === 'assistant' ? (
                <AssistantContent content={message.content} parts={message.parts} />
              ) : (
                <p>{message.content}</p>
              )}
            </div>
          </article>
          {message.id === lastUserMessageId && notice ? (
            <div className={styles.noticeRow}>
              <span className={styles.notice}>{notice}</span>
              {!running ? (
                <button className={styles.retryButton} onClick={onRetry} type="button">
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M3 12a9 9 0 1 0 2.6-6.4M3 4v5h5" />
                  </svg>
                  重新发送
                </button>
              ) : null}
            </div>
          ) : null}
        </Fragment>
      ))}
      {running || streamText ? (
        <article className={`${styles.message} ${styles.assistantMessage}`}>
          <AssistantAvatar />
          <div className={styles.messageBody}>
            <span className={styles.messageRole}>
              {providerName}
              {running ? ' · 输出中' : ''}
            </span>
            <AssistantContent content={streamText} parts={runParts} />
            {liveThinking ? (
              <div className={styles.activityLine}>
                <span className={styles.activityKind}>思考</span>
                <span>{liveThinking}</span>
              </div>
            ) : null}
            {running && !liveThinking && runParts.length === 0 ? (
              <div aria-live="polite" className={styles.executionStatus} role="status">
                <span className={styles.executionDot} />
                <span>正在思考</span>
              </div>
            ) : null}
          </div>
        </article>
      ) : null}
      {pendingApproval ? (
        <section aria-label="AI 命令确认" className={styles.approvalCard}>
          <div className={styles.approvalHeader}>
            <span>确认执行命令</span>
            <span className={styles.approvalTarget}>
              {pendingApproval.targetKind === 'ssh' ? 'SSH' : '本地'} ·{' '}
              {pendingApproval.targetLabel}
            </span>
          </div>
          <pre className={styles.approvalCommand}>{pendingApproval.command}</pre>
          <p>该命令不属于自动允许的只读范围，仅本次允许后才会执行。</p>
          <div className={styles.approvalActions}>
            <button
              disabled={approvalSubmitting}
              onClick={() => onResolveApproval(false)}
              type="button"
            >
              拒绝
            </button>
            <button
              className={styles.approveButton}
              disabled={approvalSubmitting}
              onClick={() => onResolveApproval(true)}
              type="button"
            >
              {approvalSubmitting ? '提交中…' : '允许一次'}
            </button>
          </div>
        </section>
      ) : null}
      {notice && !lastUserMessageId ? <div className={styles.notice}>{notice}</div> : null}
    </div>
  );
}

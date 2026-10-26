import { useTerminalStore } from '../../terminal';
import {
  useAiPanelController,
  type AiPanelControllerInput,
} from '../model/use-ai-panel-controller';
import { AiComposer } from './AiComposer';
import { AiConversationView } from './AiConversationView';
import { AiHistoryMenu } from './AiHistoryMenu';
import { AiPanelToggle } from './AiPanelToggle';
import { AiTaskMenu } from './AiTaskMenu';
import styles from './AiPanel.module.css';

interface AiPanelProps {
  activeView: AiPanelControllerInput['activeView'];
  onActivateTerminal: AiPanelControllerInput['onActivateTerminal'];
  onTerminalSelectionHandled: AiPanelControllerInput['onTerminalSelectionHandled'];
  terminalSelectionRequest: AiPanelControllerInput['terminalSelectionRequest'];
  visible: boolean;
}

export type { AiTerminalSelectionRequest } from '../model/use-ai-panel-controller';

/** AI 工作台渲染层：业务编排全部收拢在 useAiPanelController，本组件只消费返回值。 */
export function AiPanel({
  activeView,
  onActivateTerminal,
  onTerminalSelectionHandled,
  terminalSelectionRequest,
  visible,
}: AiPanelProps) {
  const sessions = useTerminalStore((state) => state.sessions);
  const activeId = useTerminalStore((state) => state.activeId);
  const recentRemoteSessionIds = useTerminalStore((state) => state.recentRemoteSessionIds);
  const terminalStatuses = useTerminalStore((state) => state.statuses);

  const controller = useAiPanelController({
    activeId,
    activeView,
    onActivateTerminal,
    onTerminalSelectionHandled,
    recentRemoteSessionIds,
    sessions,
    terminalSelectionRequest,
    terminalStatuses,
    visible,
  });
  const {
    activeConversation,
    activeSessionStatus,
    activeTask,
    attachmentItems,
    attachmentLoading,
    backgroundTasks,
    composerRef,
    contextText,
    currentTargetBusyElsewhere,
    draft,
    fileInputRef,
    historyOpen,
    lastUserMessageId,
    messages,
    provider,
    providerAvailability,
    providerMenuOpen,
    selectedProvider,
    targetReady,
    targetUnavailableMessage,
    taskAttentionBadge,
    taskAttentionCount,
    taskMenuOpen,
    currentTargetTask,
    viewingCurrentTarget,
    visibleNotice,
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
    requestSessionStop,
    resolveApproval,
    retryLastQuestion,
    setCommandPolicy,
    setHistoryOpen,
    setProviderMenuOpen,
    setTaskMenuOpen,
    setOpen,
    submit,
  } = controller;

  if (!visible) return null;

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
              setHistoryOpen(!historyOpen);
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
              aria-expanded={Boolean(taskMenuOpen) && taskAttentionCount > 0}
              aria-label={`${taskAttentionCount} 个需要关注的 AI 任务`}
              className={`${styles.iconButton} ${styles.taskButton} ${
                backgroundTasks.some((task) => task.pendingApproval) ? styles.taskButtonWaiting : ''
              }`}
              onClick={() => {
                setTaskMenuOpen(!taskMenuOpen);
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
            disabled={!controller.canBeginConversation}
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
          activeConversationId={controller.activeConversationId}
          canClear={messages.length > 0}
          conversations={controller.conversations}
          currentTargetKey={workspaceTargetKey}
          onClear={() => void handleClearConversation()}
          onDelete={(id) => void handleDeleteConversation(id)}
          onSelect={(id) => void handleSelectConversation(id)}
          runningConversationIds={controller.runningConversationIds}
        />
      ) : null}

      {taskMenuOpen && taskAttentionCount ? (
        <AiTaskMenu
          activeConversationId={controller.activeConversationId}
          completedResults={controller.unreadTaskResults}
          onMarkAllRead={controller.markAllTaskResultsRead}
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
        attachments={attachmentItems}
        attachmentLoading={attachmentLoading}
        commandPolicy={controller.commandPolicy}
        composerDisabled={
          !viewingCurrentTarget || !workspaceTargetKey || currentTargetBusyElsewhere
        }
        composerRef={composerRef}
        controlsDisabled={Boolean(currentTargetTask)}
        draft={draft}
        fileInputRef={fileInputRef}
        onAttachmentChange={handleAttachmentChange}
        onAttachmentRemove={removeCurrentAttachment}
        onCommandPolicyChange={(policy) => {
          if (workspaceTargetKey) {
            setCommandPolicy(workspaceTargetKey, controller.activeConversationId, policy);
          }
        }}
        onComposerKeyDown={handleComposerKeyDown}
        onDraftChange={handleDraftChange}
        onProviderChange={(nextProvider) => void handleProviderChange(nextProvider)}
        onProviderMenuToggle={() => setProviderMenuOpen(!providerMenuOpen)}
        onStop={() => {
          if (activeTask) requestSessionStop(activeTask.sessionId);
        }}
        onSubmit={submit}
        provider={selectedProvider}
        providerAvailable={providerAvailability[provider]}
        providerId={provider}
        providerMenuOpen={Boolean(providerMenuOpen)}
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

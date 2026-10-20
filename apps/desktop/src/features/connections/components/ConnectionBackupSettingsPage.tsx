import { useEffect, useState } from 'react';

import { ConnectionBackupMenu } from './ConnectionBackupMenu';
import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import {
  bindConnectionPrivateKeyFile,
  importConnectionBackup,
  normalizeConnectionError,
  pickConnectionBackup,
  pickSshConfig,
  resolveLocalUsername,
  resolveSshConfigPrivateKeyPath,
  saveConnectionBackup,
} from '../api/connection-client';
import { createConnectionBackup, parseConnectionBackup } from '../model/connection-backup';
import { parseSshConfigConnections } from '../model/ssh-config-import';
import { useConnections } from '../model/useConnections';
import styles from './ConnectionBackupSettingsPage.module.css';

const FEEDBACK_TIMEOUT_MS = 5_000;

/**
 * 备份页面只迁移原连接侧栏的入口，不扩展为数据库快照或云同步。
 * JSON 仍只包含连接与分组；密码、私钥内容和本机私钥路径不会写入文件。
 */
export function ConnectionBackupSettingsPage() {
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const { connections, groups, loading, error, reload } = useConnections();

  const clearFeedback = () => {
    setActionError(null);
    setActionMessage(null);
  };

  const requireDesktopRuntime = () => {
    if (isDesktopRuntime()) return true;
    setActionMessage(null);
    setActionError('请在 Nocterm 桌面客户端中使用备份与恢复');
    return false;
  };

  useEffect(() => {
    if (!actionError && !actionMessage) return;
    const timeout = window.setTimeout(clearFeedback, FEEDBACK_TIMEOUT_MS);
    return () => window.clearTimeout(timeout);
  }, [actionError, actionMessage]);

  const handleExportConnections = async () => {
    if (!requireDesktopRuntime()) return;
    clearFeedback();
    setBusy(true);
    try {
      const defaultPath = `nocterm-connections-${new Date().toISOString().slice(0, 10)}.json`;
      const saved = await saveConnectionBackup(
        createConnectionBackup(connections, groups),
        defaultPath
      );
      if (saved) setActionMessage('连接配置已导出。敏感凭据不会写入备份文件。');
    } catch (backupError) {
      setActionError(`导出连接配置失败：${normalizeConnectionError(backupError).message}`);
    } finally {
      setBusy(false);
    }
  };

  const handleImportConnections = async () => {
    if (!requireDesktopRuntime()) return;
    clearFeedback();
    setBusy(true);
    try {
      const content = await pickConnectionBackup();
      if (!content) return;
      const result = await importConnectionBackup(parseConnectionBackup(content));
      await reload();
      setActionMessage(
        `已导入 ${result.connections} 个连接、${result.groups} 个分组。敏感凭据需要在本机重新绑定。`
      );
    } catch (backupError) {
      setActionError(`导入连接配置失败：${normalizeConnectionError(backupError).message}`);
    } finally {
      setBusy(false);
    }
  };

  const handleImportSshConfig = async () => {
    if (!requireDesktopRuntime()) return;
    clearFeedback();
    setBusy(true);
    try {
      const selected = await pickSshConfig();
      if (!selected) return;
      const group = groups.find((item) => item.name === 'SSH Config');
      const groupId = group?.id ?? `group-${crypto.randomUUID()}`;
      const imported = parseSshConfigConnections(
        selected.content,
        groupId,
        selected.path,
        await resolveLocalUsername()
      );
      if (imported.length === 0) throw new Error('未找到可导入的 Host 配置');

      const result = await importConnectionBackup({
        groups: group ? [] : [{ id: groupId, name: 'SSH Config', sortOrder: groups.length }],
        connections: imported.map((connection) => ({
          sourceId: connection.sourceId,
          name: connection.name,
          host: connection.host,
          port: connection.port,
          username: connection.username,
          authentication: connection.authentication,
          groupId: connection.groupId,
          remark: connection.remark,
          remoteInitialPath: connection.remoteInitialPath,
          icon: connection.icon,
          sortOrder: connection.sortOrder,
          credentialKind: connection.credentialKind,
          credentialStatus: connection.credentialStatus,
        })),
      });
      const importedIds = new Map(
        result.importedConnections.map((connection) => [connection.sourceId, connection.id])
      );
      let unboundPrivateKeys = 0;
      for (const connection of imported) {
        if (!connection.privateKeyPath) continue;
        const connectionId = importedIds.get(connection.sourceId);
        const privateKeyPath = await resolveSshConfigPrivateKeyPath(connection.privateKeyPath);
        if (connectionId === undefined || !privateKeyPath) {
          unboundPrivateKeys += 1;
          continue;
        }
        try {
          await bindConnectionPrivateKeyFile(connectionId, privateKeyPath);
        } catch {
          // 主体导入已提交；本机私钥不可用时保留连接，交给用户重新绑定。
          unboundPrivateKeys += 1;
        }
      }
      await reload();
      setActionMessage(
        unboundPrivateKeys > 0
          ? `已从 SSH Config 导入 ${result.connections} 个连接，${unboundPrivateKeys} 个私钥需要手动绑定。`
          : `已从 SSH Config 导入 ${result.connections} 个连接。`
      );
    } catch (backupError) {
      setActionError(`导入 SSH Config 失败：${normalizeConnectionError(backupError).message}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={styles.page}>
      <header className={styles.pageHeader}>
        <h1>备份与恢复</h1>
      </header>

      <section className={styles.section} aria-labelledby="connection-backup-heading">
        <div className={styles.sectionHeading}>
          <h2 id="connection-backup-heading">连接配置</h2>
          <span>{loading ? '正在读取…' : `${connections.length} 个连接`}</span>
        </div>

        {error ? (
          <div className={`${styles.feedback} ${styles.error}`} role="alert">
            <span>{error.message}</span>
            <button onClick={() => void reload()} type="button">
              重试
            </button>
          </div>
        ) : null}

        <ConnectionBackupMenu
          busy={busy || loading}
          onExportConnections={() => void handleExportConnections()}
          onImportConnections={() => void handleImportConnections()}
          onImportSshConfig={() => void handleImportSshConfig()}
        />
      </section>

      {actionError ? (
        <div className={`${styles.feedback} ${styles.error}`} role="alert">
          {actionError}
        </div>
      ) : null}
      {actionMessage ? (
        <div className={`${styles.feedback} ${styles.success}`} role="status">
          {actionMessage}
        </div>
      ) : null}
    </div>
  );
}

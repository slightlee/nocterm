import type { FileEditorSource } from './file-editor-store';

interface DirectorySnapshot {
  enabled: boolean;
  connectionId?: string;
  localPath?: string;
  remotePath: string;
}

/** 路径来自后端身份；同时支持 Windows 分隔符，根目录保留分隔符。 */
export function fileParentPath(path: string): string | null {
  const index = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  if (index < 0) return null;
  if (index === 0 || (index === 2 && path[1] === ':')) return path.slice(0, index + 1);
  return path.slice(0, index);
}

/** 保存发生在旧标签也必须失效缓存，只有当前可见且身份相同的目录才立即刷新。 */
export function refreshSavedFile(
  source: FileEditorSource,
  current: DirectorySnapshot,
  actions: {
    invalidateLocalPath: (path: string) => void;
    invalidateRemotePath: (connectionId: string, path: string) => void;
    loadLocalDirectory: (path: string, force: boolean) => Promise<void>;
    loadRemoteDirectory: (path: string, force: boolean) => Promise<void>;
  }
) {
  const parent = fileParentPath(source.path);
  if (!parent) return;
  if (source.kind === 'local') {
    // 即使用户已离开本地目录，也要清缓存，避免切回后显示旧大小和时间。
    actions.invalidateLocalPath(parent);
    if (current.enabled && current.localPath === parent)
      void actions.loadLocalDirectory(parent, true);
  } else {
    // 两台服务器常有同名目录；路径相同不能视为同一个刷新目标。
    // 非当前连接只失效缓存，不调用当前面板的加载函数。
    actions.invalidateRemotePath(source.connectionId, parent);
    if (
      current.enabled &&
      current.connectionId === source.connectionId &&
      current.remotePath === parent
    ) {
      void actions.loadRemoteDirectory(parent, true);
    }
  }
}

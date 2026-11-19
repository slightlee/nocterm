import * as fileApi from '../api/sftp-client';
import { getSftpErrorMessage } from './sftp-error';
import {
  canChangeEditorContent,
  fileEditorId,
  fileEditorName,
  useFileEditorStore,
  type FileEditorBaseline,
  type FileEditorSource,
  type FileEditorTab,
} from './file-editor-store';

interface EditorControllerOptions {
  onFileSaved: (source: FileEditorSource) => void;
  showToast: (message: string, tone?: 'error' | 'info' | 'success') => void;
}

function baselineOf(
  value: fileApi.LocalFileBaseline | fileApi.RemoteSaveResult
): FileEditorBaseline {
  // 本地 mtime 保留纳秒；远程版本还必须带回后端签发的目标身份。
  // 两种基线只在展示状态中统一，发出 IPC 时仍按各自协议拆分。
  return 'modifiedNanos' in value
    ? { size: value.size, mtimeSeconds: value.modifiedSeconds, mtimeNanos: value.modifiedNanos }
    : {
        size: value.size,
        mtimeSeconds: value.modifiedAt,
        mtimeNanos: null,
        targetRevision: value.targetRevision,
      };
}

/** 请求身份由 Store 的打开实例持有，所有关闭入口均能使旧请求失效。 */
export function createFileEditorController({ onFileSaved, showToast }: EditorControllerOptions) {
  const current = (id: string) =>
    useFileEditorStore.getState().editors.find((editor) => editor.id === id);
  const isCurrent = (editor: FileEditorTab) => current(editor.id)?.instanceId === editor.instanceId;
  // patch 不按路径单独匹配：路径相同但 instance 不同代表用户已关闭并重开。
  // 这让标签关闭、父连接关闭和异步失败都遵守同一结果失效规则。
  const patch = (editor: FileEditorTab, values: Partial<FileEditorTab>) => {
    if (isCurrent(editor)) useFileEditorStore.getState().patchEditor(editor.id, values);
  };

  async function read(source: FileEditorSource) {
    // 来源决定 API；本地返回路径用于身份去重，不借用当前可见面板。
    if (source.kind === 'local') {
      const loaded = await fileApi.readLocalFile(source.path);
      return {
        content: loaded.content,
        source: { ...source, path: loaded.identityPath },
        baseline: baselineOf(loaded.baseline),
      };
    }
    const loaded = await fileApi.readRemoteFile(source.connectionId, source.path);
    return { content: loaded.content, source, baseline: baselineOf(loaded) };
  }

  async function openFile(source: FileEditorSource) {
    const state = useFileEditorStore.getState();
    if (state.closePending) return;
    const id = fileEditorId(source);
    if (current(id)) {
      // 重复打开只聚焦，不能覆盖该标签的草稿或重新发出读取。
      state.activateEditor(id);
      return;
    }
    state.openEditor({
      id,
      name: fileEditorName(source.path),
      source,
      content: '',
      savedContent: '',
      baseline: { size: null, mtimeSeconds: null, mtimeNanos: null },
    });
    const opened = current(id)!;
    // 实例在首次 await 前固定，结果只能回写同一次打开。
    try {
      const loaded = await read(source);
      // 身份升级与填充在一次 Store 更新内完成，关闭后不能复活旧标签。
      useFileEditorStore.getState().completeOpen(id, opened.instanceId, loaded.source, {
        content: loaded.content,
        savedContent: loaded.content,
        baseline: loaded.baseline,
      });
    } catch (error) {
      if (!isCurrent(opened)) return;
      useFileEditorStore.getState().closeEditor(id);
      showToast(`打开文件失败：${getSftpErrorMessage(error)}`);
    }
  }

  async function saveFile(id: string, force = false) {
    const editor = current(id);
    if (!editor || editor.status !== 'ready' || useFileEditorStore.getState().closePending) return;
    if (editor.targetInvalidated) {
      // 仍允许复制和编辑草稿，只禁止向变更后的目标继续上传。
      showToast('连接已变更，请复制未保存内容后关闭并重新打开此文件。');
      return;
    }
    if (!editor.dirty && !force) return;
    patch(editor, { status: 'saving' });
    // editor 是本次上传的不可变快照；保存期间用户仍可在 Store 中继续输入。
    // 成功后只推进 savedContent，并用最新草稿重新计算 dirty。
    try {
      const baseline = baselineOf(
        editor.source.kind === 'local'
          ? await fileApi.saveLocalFile(
              editor.source.path,
              editor.content,
              {
                size: editor.baseline.size!,
                modifiedSeconds: editor.baseline.mtimeSeconds!,
                modifiedNanos: editor.baseline.mtimeNanos!,
              },
              force
            )
          : await fileApi.saveRemoteFile(
              editor.source.connectionId,
              editor.source.path,
              editor.content,
              {
                size: editor.baseline.size,
                modifiedAt: editor.baseline.mtimeSeconds,
                targetRevision: editor.baseline.targetRevision!,
              },
              force
            )
      );
      // 保存通知与标签是否仍可见无关；浏览器自行检查当前连接和路径。
      onFileSaved(editor.source);
      if (!isCurrent(editor)) return;
      patch(editor, {
        baseline,
        status: 'ready',
        savedContent: editor.content,
        dirty: current(id)!.content !== editor.content,
        conflictPending: false,
      });
      showToast('已保存', 'success');
    } catch (error) {
      if (!isCurrent(editor)) return;
      const code = (error as { code?: string } | null)?.code;
      // 目标变更不能交给“覆盖保存”绕过。普通内容冲突才进入三选对话。
      // 其它失败保留草稿并恢复 ready，允许修复环境后再次保存。
      patch(editor, { status: 'ready' });
      if (code === 'SFTP_REMOTE_TARGET_CHANGED') patch(editor, { targetInvalidated: true });
      else if (code === 'SFTP_REMOTE_CHANGED' || code === 'SFTP_LOCAL_FILE_CHANGED') {
        patch(editor, { conflictPending: true });
        return;
      }
      showToast(`保存失败：${getSftpErrorMessage(error)}`);
    }
  }

  async function reloadFile(id: string) {
    const editor = current(id);
    if (
      !editor ||
      editor.status !== 'ready' ||
      editor.targetInvalidated ||
      useFileEditorStore.getState().closePending
    )
      return;
    patch(editor, { status: 'loading' });
    // 加载最新版本由冲突对话显式触发；读失败不能清空原草稿。
    try {
      const loaded = await read(editor.source);
      if (!isCurrent(editor)) return;
      // 重载也校验目标版本，不能在旧编辑标签内装入另一台服务器的内容。
      if (
        editor.source.kind === 'remote' &&
        loaded.baseline.targetRevision !== editor.baseline.targetRevision
      ) {
        patch(editor, { status: 'ready', targetInvalidated: true });
        showToast('连接已变更，请复制未保存内容后关闭并重新打开此文件。');
        return;
      }
      patch(editor, {
        content: loaded.content,
        savedContent: loaded.content,
        baseline: loaded.baseline,
        status: 'ready',
        dirty: false,
        conflictPending: false,
      });
      showToast('已加载最新版本，未保存修改已丢弃', 'info');
    } catch (error) {
      if (!isCurrent(editor)) return;
      patch(editor, { status: 'ready' });
      showToast(`加载最新版本失败：${getSftpErrorMessage(error)}`);
    }
  }

  return {
    openFile,
    saveFile,
    reloadFile,
    changeContent(id: string, value: string) {
      const editor = current(id);
      if (
        !editor ||
        !canChangeEditorContent(editor.status) ||
        useFileEditorStore.getState().closePending
      )
        return;
      patch(editor, { content: value, dirty: value !== editor.savedContent });
    },
    async resolveConflict(id: string, choice: 'overwrite' | 'reload' | 'cancel') {
      // 取消只收起对话；加载与覆盖均遵守各自的请求和目标保护。
      const editor = current(id);
      if (!editor) return;
      patch(editor, { conflictPending: false });
      if (choice === 'overwrite') await saveFile(id, true);
      if (choice === 'reload') await reloadFile(id);
    },
  };
}

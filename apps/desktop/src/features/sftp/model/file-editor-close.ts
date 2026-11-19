import type { ConfirmState } from '../types/sftp-types';
import { useFileEditorStore } from './file-editor-store';

type Confirm = (options: Omit<ConfirmState, 'resolve'>) => Promise<boolean>;
type Notice = (message: string) => void;

/** 关闭确认至副作用结束期间冻结新编辑操作，避免确认后又开始保存或打开子标签。 */
export async function beginFileEditorClose(
  ids: readonly string[],
  confirm: Confirm,
  notice: Notice
): Promise<(() => void) | null> {
  const store = useFileEditorStore.getState();
  if (store.closePending) {
    notice('正在处理另一个文件关闭操作，请稍后重试。');
    return null;
  }
  store.setClosePending(true);
  // release 的责任交给真正执行关闭的调用方，不能在确认框返回时提前解冻。
  const release = () => useFileEditorStore.getState().setClosePending(false);
  let accepted = false;
  try {
    // 保存中拒绝关闭，不把“取消传输”当作已保存，也不丢弃尚未返回的写入结果。
    const editors = store.editors.filter((editor) => ids.includes(editor.id));
    if (editors.some((editor) => editor.status === 'saving')) {
      notice('文件正在保存，请等待保存完成后再关闭。');
      return null;
    }
    const count = editors.filter((editor) => editor.dirty).length;
    if (
      count &&
      !(await confirm({
        title: '放弃未保存修改',
        message: `${count} 个文件尚未保存，继续将丢失修改。`,
        confirmLabel: '放弃修改并继续',
        danger: true,
      }))
    )
      return null;
    accepted = true;
    return release;
  } finally {
    // 拒绝、取消及确认框异常都必须恢复交互，不能留下常驻全局锁。
    if (!accepted) release();
  }
}

/** 删除连接与退出应用复用标签的同一保护；失败时内容和关闭能力都保留。 */
export async function withFileEditorClose(
  ids: readonly string[],
  confirm: Confirm,
  notice: Notice,
  action: () => void | Promise<void>
): Promise<boolean> {
  const release = await beginFileEditorClose(ids, confirm, notice);
  if (!release) return false;
  try {
    await action();
    return true;
  } finally {
    // 断开连接或系统关闭失败时也释放锁；草稿是否移除由成功的副作用负责。
    release();
  }
}

export function remoteEditorIds(connectionId: string) {
  return useFileEditorStore
    .getState()
    .editors.filter(
      (editor) => editor.source.kind === 'remote' && editor.source.connectionId === connectionId
    )
    .map((editor) => editor.id);
}

/** 连接修改保留草稿；锁只阻止读写与配置更新交错，不要求丢弃内容。 */
export async function withFileEditorTargetChange<T>(
  connectionId: string,
  action: () => Promise<T>
): Promise<T> {
  const store = useFileEditorStore.getState();
  const ids = remoteEditorIds(connectionId);
  if (
    store.closePending ||
    store.editors.some((editor) => ids.includes(editor.id) && editor.status === 'saving')
  ) {
    throw new Error('文件仍在保存或关闭，请完成后再修改连接。');
  }
  store.setClosePending(true);
  try {
    const result = await action();
    useFileEditorStore.getState().invalidateConnection(connectionId);
    // 配置保存失败不失效旧编辑器；后端仍会校验真正的目标版本。
    return result;
  } finally {
    useFileEditorStore.getState().setClosePending(false);
  }
}

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ask, message } from '@tauri-apps/plugin-dialog';
import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';

/** 先监听再启用保护；系统退出不依赖前端异步上报的 dirty 数量。 */
export async function onFileEditorCloseRequested(handler: (id: number) => void) {
  if (!isDesktopRuntime()) return () => {};
  const unlisten = await listen<number>('nocterm://file-editor-close-requested', (event) =>
    handler(event.payload)
  );
  try {
    await invoke('file_editor_enable_close_guard');
  } catch (error) {
    unlisten();
    throw error;
  }
  return unlisten;
}

export function resolveFileEditorClose(requestId: number, allow: boolean) {
  // 请求 ID 由后端产生；不能通过一次同意永久授权未来的系统关闭动作。
  return invoke<void>('file_editor_resolve_close', { requestId, allow });
}

export function confirmEditorDiscard(text: string): Promise<boolean> {
  // 使用原生对话使 macOS 菜单退出与 Windows 标题栏关闭具有相同确认语义。
  if (!isDesktopRuntime()) return Promise.resolve(window.confirm(text));
  return ask(text, {
    title: '放弃未保存修改',
    kind: 'warning',
    okLabel: '放弃修改并继续',
    cancelLabel: '取消',
  });
}

export function showEditorCloseNotice(text: string) {
  // 保存中的关闭请求已经在后端被阻止，此处只解释原因，不触发第二次退出。
  if (isDesktopRuntime()) {
    void message(text, { title: '暂时无法关闭', kind: 'warning' }).catch(() => {
      console.error('File editor close notice failed');
    });
  }
}

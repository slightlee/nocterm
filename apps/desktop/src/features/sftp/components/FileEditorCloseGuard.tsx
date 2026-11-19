import { useEffect } from 'react';
import {
  confirmEditorDiscard,
  onFileEditorCloseRequested,
  resolveFileEditorClose,
  showEditorCloseNotice,
} from '../api/file-editor-lifecycle-client';
import { withFileEditorClose } from '../model/file-editor-close';
import { useFileEditorStore } from '../model/file-editor-store';

/** 常驻应用壳，设置页和隐藏编辑器也受到相同的系统退出保护。 */
export function FileEditorCloseGuard() {
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void onFileEditorCloseRequested((id) => {
      // StrictMode 的旧订阅可能在异步注销前收到一次事件，不能替新实例取消确认。
      if (disposed) return;
      void (async () => {
        try {
          // 在关闭事件到达后读取 Store，包含最后一个按键刚产生的草稿。
          // 不依赖异步镜像给 Rust 的 dirty 计数，避免时序窗口导致静默退出。
          const ids = useFileEditorStore.getState().editors.map((editor) => editor.id);
          const accepted = await withFileEditorClose(
            ids,
            (options) => confirmEditorDiscard(options.message),
            showEditorCloseNotice,
            () => resolveFileEditorClose(id, true)
          );
          if (!accepted) await resolveFileEditorClose(id, false);
        } catch (error) {
          console.error('File editor close request failed', error);
          try {
            await resolveFileEditorClose(id, false);
          } catch {
            console.error('File editor close cancellation failed');
          }
          showEditorCloseNotice('关闭失败，请稍后重试。');
        }
      })();
    })
      .then((dispose) => {
        // 监听建立可能晚于 StrictMode 卸载，资源归属仍由本次 effect 负责。
        if (disposed) dispose();
        else unlisten = dispose;
      })
      .catch(() => {
        console.error('File editor close guard subscription failed');
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
  return null;
}

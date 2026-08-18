const OPEN_CONNECTION_DIALOG_EVENT = 'nocterm:open-connection-dialog';

/** 跨布局触发新建弹窗，避免终端模块依赖连接列表内部状态。 */
export function requestConnectionDialog() {
  window.dispatchEvent(new Event(OPEN_CONNECTION_DIALOG_EVENT));
}

export function onConnectionDialogRequested(listener: () => void) {
  window.addEventListener(OPEN_CONNECTION_DIALOG_EVENT, listener);
  return () => window.removeEventListener(OPEN_CONNECTION_DIALOG_EVENT, listener);
}

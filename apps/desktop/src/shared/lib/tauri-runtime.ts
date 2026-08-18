/** 浏览器预览不伪造持久化结果，仅用于展示不可用状态和检查界面。 */
export function isDesktopRuntime(): boolean {
  return '__TAURI_INTERNALS__' in window;
}

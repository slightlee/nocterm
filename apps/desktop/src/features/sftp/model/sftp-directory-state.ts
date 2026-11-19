/** 目录请求对 loading 的最小状态事件，供本地/远程浏览器共享并纯函数测试。 */
export type DirectoryLoadingEvent =
  'request-started' | 'cache-hit' | 'request-unavailable' | 'current-request-settled';

/**
 * 缓存命中代表当前请求已有可展示结果，必须主动清除 loading；旧请求的 finally
 * 因 requestId 过期不会收尾。过期请求结束时则保持当前状态，不能清掉新请求。
 */
export function nextDirectoryLoading(current: boolean, event: DirectoryLoadingEvent): boolean {
  switch (event) {
    case 'request-started':
      return true;
    case 'cache-hit':
    case 'request-unavailable':
    case 'current-request-settled':
      return false;
  }
}

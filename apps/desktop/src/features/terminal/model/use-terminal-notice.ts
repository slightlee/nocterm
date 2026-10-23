import { useCallback, useEffect, useRef, useState } from 'react';

const NOTICE_DURATION_MS = 5_200;

/** 剪贴板失败使用界面提示，不向 PTY 注入伪终端输出。 */
export function useTerminalNotice() {
  const [notice, setNotice] = useState<string | null>(null);
  const timerRef = useRef<number | null>(null);

  const showNotice = useCallback((message: string) => {
    if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    setNotice(message);
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null;
      setNotice(null);
    }, NOTICE_DURATION_MS);
  }, []);

  useEffect(
    () => () => {
      if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    },
    []
  );

  return { notice, showNotice };
}

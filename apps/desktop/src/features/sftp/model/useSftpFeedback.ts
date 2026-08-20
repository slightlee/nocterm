import { useCallback, useRef, useState } from 'react';
import type { ConfirmState, ToastState } from '../types/sftp-types';

export function useSftpFeedback() {
  const [toasts, setToasts] = useState<ToastState[]>([]);
  const [confirmState, setConfirmState] = useState<ConfirmState | null>(null);
  const toastIdRef = useRef(0);

  const showToast = useCallback((message: string, tone: ToastState['tone'] = 'error') => {
    const id = toastIdRef.current++;
    setToasts((current) => [...current, { id, tone, message }]);
    window.setTimeout(
      () => {
        setToasts((current) => current.filter((toast) => toast.id !== id));
      },
      tone === 'error' ? 5200 : 3200
    );
  }, []);

  const requestConfirm = useCallback(
    (options: Omit<ConfirmState, 'resolve'>) =>
      new Promise<boolean>((resolve) => {
        setConfirmState({ ...options, resolve });
      }),
    []
  );

  const closeConfirm = useCallback((confirmed: boolean) => {
    setConfirmState((current) => {
      current?.resolve(confirmed);
      return null;
    });
  }, []);

  return {
    toasts,
    confirmState,
    showToast,
    requestConfirm,
    closeConfirm,
  };
}

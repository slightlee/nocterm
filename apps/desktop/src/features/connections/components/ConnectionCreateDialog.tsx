import { useEffect, useRef, useState, type FormEvent } from 'react';
import * as DialogPrimitive from '@radix-ui/react-dialog';

import { selectPrivateKeyFile } from '../api/connection-client';
import {
  DEFAULT_CONNECTION_FORM,
  toConnectionCreateRequest,
  validateConnectionForm,
  type ConnectionFieldErrors,
  type ConnectionFormValues,
} from '../model/connection-form';
import { CONNECTION_ICON_OPTIONS, getConnectionIconOption } from '../model/connection-icons';
import type { AppError, ConnectionCreateRequest, ConnectionGroup } from '../types/connection-types';
import styles from './ConnectionCreateDialog.module.css';

interface ConnectionCreateDialogProps {
  saving: boolean;
  onClose: () => void;
  onCreate: (request: ConnectionCreateRequest) => Promise<unknown>;
  initialValues?: Partial<ConnectionFormValues>;
  editing?: boolean;
  groups?: ConnectionGroup[];
  credentialNotice?: string;
  credentialBound?: boolean;
}

export function ConnectionCreateDialog({
  saving,
  onClose,
  onCreate,
  initialValues,
  editing = false,
  groups = [],
  credentialNotice,
  credentialBound = false,
}: ConnectionCreateDialogProps) {
  const [values, setValues] = useState<ConnectionFormValues>({
    ...DEFAULT_CONNECTION_FORM,
    ...initialValues,
  });
  const [errors, setErrors] = useState<ConnectionFieldErrors>({});
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [iconPickerOpen, setIconPickerOpen] = useState(false);
  const nameInputRef = useRef<HTMLInputElement>(null);
  const iconPickerRef = useRef<HTMLDivElement>(null);
  const selectedIcon = getConnectionIconOption(values.icon);

  useEffect(() => {
    nameInputRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!iconPickerOpen) return;
    const closeIconPicker = (event: PointerEvent) => {
      if (!iconPickerRef.current?.contains(event.target as Node)) setIconPickerOpen(false);
    };
    window.addEventListener('pointerdown', closeIconPicker);
    return () => window.removeEventListener('pointerdown', closeIconPicker);
  }, [iconPickerOpen]);

  const update = <Key extends keyof ConnectionFormValues>(
    key: Key,
    value: ConnectionFormValues[Key]
  ) => {
    // 用户修正字段时只清理当前字段错误，保留其他待处理提示。
    setValues((current) => ({ ...current, [key]: value }));
    setErrors((current) => ({ ...current, [key]: undefined }));
    setSubmitError(null);
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const nextErrors = validateConnectionForm(values);
    if (Object.keys(nextErrors).length > 0) {
      setErrors(nextErrors);
      return;
    }

    try {
      // 只有后端确认持久化成功后才关闭弹窗，失败时保留用户输入。
      await onCreate(toConnectionCreateRequest(values));
      onClose();
    } catch (error) {
      setSubmitError((error as AppError).message ?? '保存连接失败，请稍后重试');
    }
  };

  const pickPrivateKey = async () => {
    try {
      const selected = await selectPrivateKeyFile();
      if (selected) update('privateKeyPath', selected);
    } catch {
      setSubmitError('无法打开私钥文件选择器，请稍后重试');
    }
  };

  return (
    <DialogPrimitive.Root
      open
      onOpenChange={(open) => {
        if (!open && !saving) onClose();
      }}
    >
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className={styles.overlay} />
        <DialogPrimitive.Content
          aria-describedby={undefined}
          className={styles.modal}
          onInteractOutside={(event) => {
            if (saving) event.preventDefault();
          }}
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            nameInputRef.current?.focus();
          }}
        >
          <header className={styles.modalHead}>
            <div className={styles.titleWrap}>
              <DialogPrimitive.Title className={styles.title}>
                {editing ? '编辑连接' : '新建连接'}
              </DialogPrimitive.Title>
            </div>
            <button
              aria-label="关闭"
              className={styles.closeBtn}
              disabled={saving}
              onClick={onClose}
              title="关闭"
              type="button"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M18 6 6 18M6 6l12 12" />
              </svg>
            </button>
          </header>

          <div className={styles.modalBody}>
            <form className={styles.form} id="connection-form" onSubmit={submit}>
              <div className={styles.nameIconRow}>
                <div className={styles.field}>
                  <label className={styles.label} htmlFor="connection-name">
                    名称 <span className={styles.required}>*</span>
                  </label>
                  <input
                    ref={nameInputRef}
                    aria-invalid={Boolean(errors.name)}
                    className={styles.input}
                    id="connection-name"
                    onChange={(event) => update('name', event.target.value)}
                    placeholder="给这个连接起个名字"
                    type="text"
                    value={values.name}
                  />
                  {errors.name ? <div className={styles.fieldError}>{errors.name}</div> : null}
                </div>

                <div className={styles.field}>
                  <span className={styles.label}>图标</span>
                  <div className={styles.iconPicker} ref={iconPickerRef}>
                    <button
                      aria-expanded={iconPickerOpen}
                      aria-label={`图标：${selectedIcon.label}`}
                      className={styles.iconSelectButton}
                      onClick={() => setIconPickerOpen((open) => !open)}
                      title={selectedIcon.label}
                      type="button"
                    >
                      <span className={styles.selectedIcon}>{selectedIcon.icon}</span>
                      <svg className={styles.iconChevron} viewBox="0 0 24 24" aria-hidden="true">
                        <path d="m6 9 6 6 6-6" />
                      </svg>
                    </button>
                    {iconPickerOpen ? (
                      <div className={styles.iconMenu} role="listbox" aria-label="连接图标">
                        {CONNECTION_ICON_OPTIONS.map((option) => (
                          <button
                            aria-label={option.label}
                            aria-pressed={values.icon === option.value}
                            className={`${styles.iconOption} ${
                              values.icon === option.value ? styles.iconOptionActive : ''
                            }`}
                            key={option.value}
                            onClick={() => {
                              update('icon', option.value);
                              setIconPickerOpen(false);
                            }}
                            title={option.label}
                            type="button"
                          >
                            {option.icon}
                          </button>
                        ))}
                      </div>
                    ) : null}
                  </div>
                </div>
              </div>

              <div className={styles.row2}>
                <div className={styles.field}>
                  <label className={styles.label} htmlFor="connection-host">
                    主机 <span className={styles.required}>*</span>
                  </label>
                  <div className={styles.inputWrap}>
                    <svg className={styles.inputIcon} viewBox="0 0 24 24" aria-hidden="true">
                      <circle cx="12" cy="12" r="9" />
                      <path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18" />
                    </svg>
                    <input
                      aria-invalid={Boolean(errors.host)}
                      autoCapitalize="none"
                      autoCorrect="off"
                      className={`${styles.input} ${styles.inputWithIcon} ${styles.mono}`}
                      id="connection-host"
                      onChange={(event) => update('host', event.target.value)}
                      placeholder="IP 或域名"
                      spellCheck={false}
                      type="text"
                      value={values.host}
                    />
                  </div>
                  {errors.host ? <div className={styles.fieldError}>{errors.host}</div> : null}
                </div>

                <div className={styles.field}>
                  <label className={styles.label} htmlFor="connection-port">
                    端口 <span className={styles.required}>*</span>
                  </label>
                  <div className={styles.inputWrap}>
                    <svg className={styles.inputIcon} viewBox="0 0 24 24" aria-hidden="true">
                      <rect x="4" y="5" width="16" height="11" rx="2" />
                      <path d="M9 20h6M12 16v4" />
                    </svg>
                    <input
                      aria-invalid={Boolean(errors.port)}
                      className={`${styles.input} ${styles.inputWithIcon} ${styles.mono}`}
                      id="connection-port"
                      max={65535}
                      min={1}
                      onChange={(event) => update('port', event.target.value)}
                      type="number"
                      value={values.port}
                    />
                  </div>
                  {errors.port ? <div className={styles.fieldError}>{errors.port}</div> : null}
                </div>
              </div>

              <div className={styles.field}>
                <label className={styles.label} htmlFor="connection-username">
                  用户名 <span className={styles.required}>*</span>
                </label>
                <div className={styles.inputWrap}>
                  <svg className={styles.inputIcon} viewBox="0 0 24 24" aria-hidden="true">
                    <circle cx="12" cy="8" r="4" />
                    <path d="M4 21c0-4 4-6 8-6s8 2 8 6" />
                  </svg>
                  <input
                    aria-invalid={Boolean(errors.username)}
                    autoCapitalize="none"
                    autoCorrect="off"
                    className={`${styles.input} ${styles.inputWithIcon} ${styles.mono}`}
                    id="connection-username"
                    onChange={(event) => update('username', event.target.value)}
                    placeholder="登录用户名"
                    spellCheck={false}
                    type="text"
                    value={values.username}
                  />
                </div>
                {errors.username ? (
                  <div className={styles.fieldError}>{errors.username}</div>
                ) : null}
              </div>

              <fieldset className={styles.authFieldset}>
                <legend className={styles.label}>认证方式</legend>
                <div className={styles.segGroup}>
                  <button
                    className={`${styles.segBtn} ${
                      values.authentication === 'password' ? styles.segActive : ''
                    }`}
                    onClick={() => update('authentication', 'password')}
                    type="button"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <rect x="5" y="11" width="14" height="9" rx="2" />
                      <path d="M8 11V8a4 4 0 0 1 8 0v3" />
                    </svg>
                    密码
                  </button>
                  <button
                    className={`${styles.segBtn} ${
                      values.authentication === 'private_key' ? styles.segActive : ''
                    }`}
                    onClick={() => update('authentication', 'private_key')}
                    type="button"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <circle cx="8" cy="15" r="4" />
                      <path d="m11 12 8-8M16 7l3 3M19 4l1.5 1.5" />
                    </svg>
                    密钥
                  </button>
                  <button
                    className={`${styles.segBtn} ${
                      values.authentication === 'ssh_agent' ? styles.segActive : ''
                    }`}
                    onClick={() => update('authentication', 'ssh_agent')}
                    type="button"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <circle cx="8" cy="15" r="4" />
                      <path d="m11 12 8-8M15 8l1.5 1.5M18 5l1.5 1.5" />
                    </svg>
                    Agent
                  </button>
                </div>
              </fieldset>

              <div className={styles.field}>
                <label className={styles.label} htmlFor="connection-group">
                  分组
                </label>
                <select
                  className={`${styles.input} ${styles.select}`}
                  id="connection-group"
                  onChange={(event) => update('groupId', event.target.value || undefined)}
                  value={values.groupId ?? ''}
                >
                  <option value="">未分组</option>
                  {groups.map((group) => (
                    <option key={group.id} value={group.id}>
                      {group.name}
                    </option>
                  ))}
                </select>
              </div>

              {values.authentication === 'password' ? (
                <div className={styles.field}>
                  <label className={styles.label} htmlFor="connection-password">
                    密码<span className={styles.labelOpt}>（可选保存）</span>
                  </label>
                  <div className={styles.inputWrap}>
                    <svg className={styles.inputIcon} viewBox="0 0 24 24" aria-hidden="true">
                      <rect x="5" y="11" width="14" height="9" rx="2" />
                      <path d="M8 11V8a4 4 0 0 1 8 0v3" />
                    </svg>
                    <input
                      autoComplete="new-password"
                      className={`${styles.input} ${styles.inputWithIcon} ${styles.mono}`}
                      id="connection-password"
                      onChange={(event) => update('password', event.target.value)}
                      placeholder={
                        credentialBound ? '留空则保留已保存密码' : '留空则连接后在终端输入'
                      }
                      type="password"
                      value={values.password ?? ''}
                    />
                  </div>
                  {credentialNotice ? (
                    <div className={styles.fieldNotice}>{credentialNotice}</div>
                  ) : null}
                </div>
              ) : null}

              {values.authentication === 'private_key' ? (
                <div className={styles.field}>
                  <label className={styles.label} htmlFor="connection-private-key">
                    私钥文件 <span className={styles.required}>*</span>
                  </label>
                  <div className={styles.inputWrap}>
                    <svg className={styles.inputIcon} viewBox="0 0 24 24" aria-hidden="true">
                      <circle cx="8" cy="15" r="4" />
                      <path d="m11 12 8-8M16 7l3 3M19 4l1.5 1.5" />
                    </svg>
                    <input
                      className={`${styles.input} ${styles.inputWithIcon} ${styles.mono}`}
                      id="connection-private-key"
                      placeholder={credentialBound ? '已绑定本机私钥' : '选择本地私钥文件'}
                      readOnly
                      type="text"
                      value={values.privateKeyPath ?? ''}
                    />
                    <button
                      className={styles.fileBtn}
                      onClick={() => void pickPrivateKey()}
                      type="button"
                    >
                      <svg viewBox="0 0 24 24" aria-hidden="true">
                        <path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2.5h6.5A2.5 2.5 0 0 1 21 10v7a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17z" />
                      </svg>
                      选择
                    </button>
                  </div>
                  {credentialNotice ? (
                    <div className={styles.fieldNotice}>{credentialNotice}</div>
                  ) : null}
                </div>
              ) : null}

              {values.authentication === 'ssh_agent' ? (
                <div className={styles.field}>
                  <span className={styles.label}>SSH Agent</span>
                  <div className={styles.agentBox}>
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <circle cx="8" cy="15" r="4" />
                      <path d="m11 12 8-8M15 8l1.5 1.5M18 5l1.5 1.5" />
                    </svg>
                    使用本机 ssh-agent
                  </div>
                </div>
              ) : null}

              <div className={styles.field}>
                <label className={styles.label} htmlFor="connection-remote-path">
                  远程初始路径<span className={styles.labelOpt}>（可选）</span>
                </label>
                <div className={styles.inputWrap}>
                  <svg className={styles.inputIcon} viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2.5h6.5A2.5 2.5 0 0 1 21 10v7a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17z" />
                  </svg>
                  <input
                    autoCapitalize="none"
                    autoCorrect="off"
                    className={`${styles.input} ${styles.inputWithIcon} ${styles.mono}`}
                    id="connection-remote-path"
                    onChange={(event) => update('remoteInitialPath', event.target.value)}
                    placeholder="连接后默认打开的目录"
                    spellCheck={false}
                    type="text"
                    value={values.remoteInitialPath ?? ''}
                  />
                </div>
              </div>

              {submitError ? <div className={styles.errorMsg}>{submitError}</div> : null}
            </form>
          </div>

          <footer className={styles.modalFoot}>
            <span className={styles.footInfo}>填写服务器信息以建立连接</span>
            <div className={styles.footActions}>
              <button className={styles.btnGhost} disabled={saving} onClick={onClose} type="button">
                取消
              </button>
              <button
                className={styles.btnPrimary}
                disabled={saving}
                form="connection-form"
                type="submit"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M12 5v14M5 12h14" />
                </svg>
                {saving ? '保存中…' : '保存'}
              </button>
            </div>
          </footer>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

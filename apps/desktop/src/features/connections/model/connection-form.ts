import type {
  AuthenticationMethod,
  ConnectionCreateRequest,
  ConnectionIcon,
} from '../types/connection-types';

export interface ConnectionFormValues {
  name: string;
  host: string;
  port: string;
  username: string;
  authentication: AuthenticationMethod;
  password?: string;
  /** 只保存私钥文件路径；表单从不持有密钥内容，避免明文进入 React 状态树。 */
  privateKeyPath?: string;
  groupId?: string;
  remoteInitialPath?: string;
  remark?: string;
  icon?: ConnectionIcon;
}

export type ConnectionFieldErrors = Partial<
  Record<keyof Pick<ConnectionFormValues, 'name' | 'host' | 'port' | 'username'>, string>
>;

export const DEFAULT_CONNECTION_FORM: ConnectionFormValues = {
  name: '',
  host: '',
  port: '22',
  username: 'root',
  authentication: 'password',
  password: '',
  remoteInitialPath: '',
  remark: '',
  icon: 'server',
};

/** 客户端校验提供即时反馈；Rust Domain 仍会重复执行安全边界校验。 */
export function validateConnectionForm(values: ConnectionFormValues): ConnectionFieldErrors {
  const errors: ConnectionFieldErrors = {};
  const port = Number(values.port);

  if (!values.name.trim()) errors.name = '请输入连接名称';
  if (!values.host.trim()) errors.host = '请输入主机地址';
  if (
    values.host.trim() &&
    (values.host.trim().startsWith('-') || hasUnsafeSshCharacters(values.host))
  ) {
    errors.host = '主机地址不能包含空白字符或以短横线开头';
  }
  if (!values.username.trim()) errors.username = '请输入用户名';
  if (
    values.username.trim() &&
    (values.username.trim().startsWith('-') || hasUnsafeSshCharacters(values.username))
  ) {
    errors.username = '用户名不能包含空白字符或以短横线开头';
  }
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    errors.port = '端口必须在 1 到 65535 之间';
  }

  return errors;
}

function hasUnsafeSshCharacters(value: string): boolean {
  return [...value].some((character) => {
    const code = character.charCodeAt(0);
    return /\s/.test(character) || code < 0x20 || code === 0x7f;
  });
}

export function toConnectionCreateRequest(values: ConnectionFormValues): ConnectionCreateRequest {
  // 规范化发生在 DTO 组装处，避免展示组件散落 trim 和数字转换。
  return {
    name: values.name.trim(),
    host: values.host.trim(),
    port: Number(values.port),
    username: values.username.trim(),
    authentication: values.authentication,
    password: values.authentication === 'password' ? values.password?.trim() : undefined,
    privateKeyPath:
      values.authentication === 'private_key' ? values.privateKeyPath?.trim() : undefined,
    groupId: values.groupId,
    remoteInitialPath: values.remoteInitialPath?.trim() || undefined,
    remark: values.remark?.trim() || undefined,
    icon: values.icon,
  };
}

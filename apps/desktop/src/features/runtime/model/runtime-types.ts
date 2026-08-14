export interface RuntimeHealth {
  service: string;
  version: string;
  platform: string;
  architecture: string;
  terminalBackend: string;
  credentialStore: string;
  sshTransport: string;
}

export type RuntimeStatus = 'checking' | 'preview' | 'ready' | 'error';

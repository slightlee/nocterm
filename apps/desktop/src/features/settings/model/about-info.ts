export function formatPlatformLabel(platform: string): string {
  if (platform === 'macos') return 'macOS';
  if (platform === 'windows') return 'Windows';
  return platform || '未知平台';
}

export function formatArchitectureLabel(architecture: string): string {
  if (architecture === 'aarch64') return 'ARM64';
  if (architecture === 'x86_64') return 'x86_64';
  return architecture || '未知架构';
}

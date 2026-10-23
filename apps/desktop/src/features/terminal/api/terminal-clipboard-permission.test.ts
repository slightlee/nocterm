import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

interface DesktopCapability {
  permissions: string[];
}

const capability = JSON.parse(
  readFileSync(new URL('../../../../src-tauri/capabilities/default.json', import.meta.url), 'utf8')
) as DesktopCapability;

describe('terminal clipboard permissions', () => {
  it('grants only the text operations used by the terminal', () => {
    expect(capability.permissions).toContain('clipboard-manager:allow-read-text');
    expect(capability.permissions).toContain('clipboard-manager:allow-write-text');
    // 不授予图片等未使用能力，减少主窗口可访问的剪贴板表面。
    expect(capability.permissions).not.toContain('clipboard-manager:default');
  });
});

import { describe, expect, it } from 'vitest';
import { detectEditorLanguage } from './editor-language';

describe('editor language detection', () => {
  it.each([
    ['config.JSON', 'json'],
    ['compose.yml', 'yaml'],
    ['config.yaml', 'yaml'],
    ['Cargo.toml', 'toml'],
    ['app.ini', 'ini'],
    ['setup.cfg', 'ini'],
    ['app.properties', 'ini'],
    ['.gitconfig', 'ini'],
    ['.npmrc', 'ini'],
    ['.env', 'shell'],
    ['.env.production', 'shell'],
    ['service.env', 'shell'],
    ['.bashrc', 'shell'],
    ['.zshrc', 'shell'],
    ['deploy.sh', 'shell'],
    ['Dockerfile', 'dockerfile'],
    ['Dockerfile.dev', 'dockerfile'],
    ['Containerfile', 'dockerfile'],
    ['main.js', 'javascript'],
    ['module.mjs', 'javascript'],
    ['module.cjs', 'javascript'],
    ['App.jsx', 'jsx'],
    ['main.ts', 'typescript'],
    ['module.mts', 'typescript'],
    ['module.cts', 'typescript'],
    ['App.tsx', 'tsx'],
    ['deploy.py', 'python'],
    ['/srv/app/config.yaml', 'yaml'],
    ['C:\\app\\config.toml', 'toml'],
  ])('recognizes %s without inspecting its contents', (name, expected) => {
    expect(detectEditorLanguage(name, '')).toBe(expected);
  });

  it.each([
    ['#!/bin/bash\necho hello', 'shell'],
    ['#!/usr/bin/env sh\r\necho hello', 'shell'],
    ['#!/usr/bin/env -S bash -e\necho hello', 'shell'],
    ['#!/usr/bin/python3.12 -u\nprint(1)', 'python'],
    ['\uFEFF#!/usr/bin/env python3\nprint(1)', 'python'],
    ['#!/usr/bin/env node\nconsole.log(1)', 'javascript'],
  ])('recognizes an extensionless script from %j', (content, expected) => {
    expect(detectEditorLanguage('custom-script', content)).toBe(expected);
  });

  it('prefers a recognized filename over shebang and ignores dots in parent paths', () => {
    expect(detectEditorLanguage('config.yaml', '#!/bin/sh')).toBe('yaml');
    expect(detectEditorLanguage('/path.json/hosts', '127.0.0.1 localhost')).toBeNull();
  });

  it.each([
    ['notes.txt', 'ordinary text'],
    ['custom.unknown', '{"key": true}'],
    ['nginx.conf', 'server { listen 80; }'],
    ['Dockerfile-not-a-match', 'FROM alpine'],
    ['hosts', '127.0.0.1 localhost'],
    ['json', '{"key": true}'],
    ['script', '#!/bin/fish'],
    ['script', '#!/usr/bin/env something python'],
    ['script', 'echo "#!/bin/sh"'],
    ['script', '#!/usr/bin/env bashful'],
    ['script', 'plain\n#!/bin/bash'],
  ])('keeps unsupported %s as plain text', (name, content) => {
    expect(detectEditorLanguage(name, content)).toBeNull();
  });
});

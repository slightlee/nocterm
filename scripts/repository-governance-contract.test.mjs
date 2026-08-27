import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

const readText = (path) => readFileSync(path, 'utf8').replaceAll('\r\n', '\n');

describe('repository governance contract', () => {
  it('routes contributors through structured and labeled issue forms', () => {
    const bugTemplate = readText('.github/ISSUE_TEMPLATE/bug.yml');
    const featureTemplate = readText('.github/ISSUE_TEMPLATE/feature.yml');
    const releaseTemplate = readText('.github/ISSUE_TEMPLATE/release.yml');
    const config = readText('.github/ISSUE_TEMPLATE/config.yml');

    assert.match(bugTemplate, /^labels:\n {2}- bug$/m);
    assert.match(featureTemplate, /^labels:\n {2}- enhancement$/m);
    assert.match(releaseTemplate, /^labels:\n {2}- release$/m);
    assert.match(releaseTemplate, /^ {4}id: provenance$/m);
    assert.match(releaseTemplate, /^ {4}id: artifacts$/m);
    assert.match(releaseTemplate, /^ {4}id: platform_acceptance$/m);
    assert.match(releaseTemplate, /^ {4}id: evidence$/m);
    assert.match(releaseTemplate, /^ {4}id: known_issues$/m);
    assert.equal(releaseTemplate.match(/checksum: <filename\.sha256>/g)?.length, 3);
    assert.match(config, /^blank_issues_enabled: false$/m);
  });

  it('defaults pull requests to non-closing references', () => {
    const pullRequestTemplate = readText('.github/pull_request_template.md');

    assert.match(pullRequestTemplate, /^Refs #$/m);
    assert.match(pullRequestTemplate, /只有 PR 合并本身即可满足 Issue 全部验收标准时/);
    assert.match(pullRequestTemplate, /仍需合并后的目标平台、发布产物或人工验收/);
    assert.doesNotMatch(pullRequestTemplate, /^Closes #$/m);
  });

  it('keeps permanent Git history in English without restricting issue language', () => {
    const workflow = readText('.github/workflows/ci.yml');
    const pullRequestTemplate = readText('.github/pull_request_template.md');
    const developmentGuide = readText('docs/development.md');
    const commitlintConfig = readText('commitlint.config.mjs');

    assert.match(workflow, /node scripts\/check-pr-title-language\.mjs/);
    assert.match(commitlintConfig, /'subject-english-ascii': \[2, 'always'\]/);
    assert.match(pullRequestTemplate, /PR 标题必须使用英文 Conventional Commits/);
    assert.match(developmentGuide, /Issue 标题和正文允许使用中文或英文/);
    assert.match(developmentGuide, /Pull Request 标题和 Git Commit 必须使用英文/);
    assert.match(developmentGuide, /Issue 与 PR 正文不执行语言检测/);
  });

  it('keeps issue closure and release acceptance semantics documented', () => {
    const developmentGuide = readText('docs/development.md');
    const releaseGuide = readText('docs/release-process.md');

    assert.match(developmentGuide, /默认使用 `Refs #123`/);
    assert.match(developmentGuide, /只有 Pull Request 合并本身即可满足 Issue 全部验收标准时/);
    assert.match(developmentGuide, /最终安装包或人工验收/);
    assert.match(releaseGuide, /每个准备分发的候选版本必须创建或维护一条“发布验收”Issue/);
    assert.match(releaseGuide, /验收 Issue 的关闭不等同于公开发布 Draft/);
  });
});

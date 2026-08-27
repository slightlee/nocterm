import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import commitlintConfig from '../commitlint.config.mjs';
import { validatePullRequestTitleLanguage } from './check-pr-title-language.mjs';

const subjectLanguageRule = commitlintConfig.plugins[0].rules['subject-english-ascii'];

describe('pull request title language', () => {
  it('accepts English Conventional Commit titles', () => {
    assert.equal(validatePullRequestTitleLanguage('feat: add SFTP transfer queue'), null);
    assert.equal(validatePullRequestTitleLanguage('fix: support macOS 14 (Intel)'), null);
  });

  it('rejects Chinese and mixed-language titles', () => {
    assert.match(validatePullRequestTitleLanguage('feat: 增加传输队列'), /English ASCII/);
    assert.match(validatePullRequestTitleLanguage('fix: 修复 SFTP timeout'), /English ASCII/);
  });

  it('rejects missing titles', () => {
    assert.match(validatePullRequestTitleLanguage(''), /required/);
    assert.match(validatePullRequestTitleLanguage(undefined), /required/);
  });
});

describe('commit subject language', () => {
  it('uses the same English rule for branch commits', () => {
    assert.equal(subjectLanguageRule({ subject: 'add SFTP transfer queue' })[0], true);
    assert.equal(subjectLanguageRule({ subject: '增加传输队列' })[0], false);
    assert.equal(commitlintConfig.rules['subject-english-ascii'][0], 2);
  });
});

'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('path');
const { shouldExcludeFromZip } = require('../server');

test('legacy zip filter excludes build logs and internal state', () => {
  const excluded = [
    ['logs', 'logs', true],
    ['logs/build.txt', 'build.txt', false],
    ['check-results', 'check-results', true],
    ['.buildtool', '.buildtool', true],
    ['build.log', 'build.log', false],
    ['process.log', 'process.log', false],
    ['crash.dmp', 'crash.dmp', false],
    ['temporary.tmp', 'temporary.tmp', false],
  ];
  for (const [relative, name, directory] of excluded) {
    assert.equal(shouldExcludeFromZip(relative, name, directory, 19), true, relative);
  }
  assert.equal(shouldExcludeFromZip(path.join('Game.exe'), 'Game.exe', false, 19), false);
});

'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { ProgressDecoder } = require('../build-progress');
test('protocol survives every byte boundary, ignores ordinary and invalid logs', () => {
  const decoder = new ProgressDecoder();
  const log = Buffer.from('DisplayProgressbar: ignored\n[TBuildToolProgress] {"version":1,"stage":"编译中文","pct":42.4}\n[TBuildToolProgress] {"version":1,"stage":"归档","pct":-1}\n[TBuildToolProgress] {"version":2,"stage":"bad","pct":100}\n[TBuildToolProgress] broken\n');
  const events = [];
  for (const byte of log) events.push(...decoder.push(Buffer.from([byte])));
  assert.deepEqual(events, [{ stage: '编译中文', pct: 42 }, { stage: '归档', pct: null }]);
});
test('percentage is bounded and does not determine build outcome', () => {
  const decoder = new ProgressDecoder();
  assert.deepEqual(decoder.push(Buffer.from('[TBuildToolProgress] {"version":1,"stage":"阶段完成","pct":200,"success":true}\n')),
    [{ stage: '阶段完成', pct: 100 }]);
});

'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');
const net = require('net');

test('legacy server exposes only the user website and user API', async t => {
  const probe = net.createServer();
  await new Promise(resolve => probe.listen(0, '127.0.0.1', resolve));
  const port = probe.address().port;
  await new Promise(resolve => probe.close(resolve));
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'tbuild-http-'));
  const config = path.join(temp, 'config.json');
  fs.writeFileSync(config, JSON.stringify({ ai: { apiKey: 'private-key' }, buildNodes: [] }));
  const child = spawn(process.execPath, [path.resolve(__dirname, '../server.js')], { stdio: 'inherit', windowsHide: true, env: { ...process.env, PORT: String(port), NO_OPEN: '1', CONFIG_PATH: config } });
  const base = `http://127.0.0.1:${port}`;
  t.after(async () => { child.kill(); if (child.exitCode === null) await new Promise(resolve => child.once('exit', resolve)); fs.rmSync(temp, { recursive: true, force: true }); });
  let ready = false;
  for (let attempt = 0; attempt < 100; attempt++) {
    try { if ((await fetch(base + '/api/user/status')).ok) { ready = true; break; } } catch {}
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  assert.ok(ready, 'server becomes ready');
  for (const route of ['/', '/workspace', '/user.css', '/user.js']) assert.equal((await fetch(base + route)).status, 200, route);
  for (const route of ['/admin/', '/admin/app.js', '/api/status', '/api/monitor', '/api/vcs/state', '/api/build/events', '/api/open-user-page']) {
    assert.equal((await fetch(base + route)).status, 404, route);
  }
  assert.equal((await fetch(base + '/api/config', { method: 'POST', body: '{}' })).status, 404);
  const publicStatus = await (await fetch(base + '/api/user/status')).text();
  assert.ok(!publicStatus.includes('private-key'));
  assert.equal((await fetch(base + '/api/user/catalog')).status, 200);
  assert.equal((await fetch(base + '/api/user/upload', { method: 'POST', body: 'x' })).status, 404);
  assert.equal((await fetch(base + '/api/user/build', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{"nodeIds": ["absent"]}' })).status, 400);
});

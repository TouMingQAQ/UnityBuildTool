'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const http = require('http');
const { createUserApi, resolveFile, buildRequest, outputRoots, publicEvent, folderZip } = require('../user-api');

test('file paths stay inside root, including junctions', async t => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'tbuild-boundary-'));
  t.after(() => fs.rmSync(temp, { recursive: true, force: true }));
  const root = path.join(temp, 'root'); fs.mkdirSync(root);
  fs.writeFileSync(path.join(root, '中文.txt'), 'ok');
  fs.mkdirSync(path.join(temp, 'outside'));
  fs.symlinkSync(path.join(temp, 'outside'), path.join(root, 'linked'), 'junction');
  assert.equal((await resolveFile(root, '中文.txt')).stat.size, 2);
  for (const file of ['../outside', 'nested/../../outside', '/outside', 'C:/secret', 'x\\..\\outside', 'linked']) await assert.rejects(resolveFile(root, file), { status: 403 });
});

test('build settings are assembled on server, signing values retained internally', () => {
  const request = buildRequest({ projectPath: 'project', engine: { template: 'secret' }, defaultDevBuild: true }, [{ id: 'one', assetPath: 'one.asset', keystorePass: 'private' }]);
  assert.equal(request.profiles[0].sign.keystorePass, 'private');
  assert.equal(request.dev, true);
  assert.equal(request.engine.template, 'secret');
  const roots = outputRoots({ projectPath: 'project', buildNodes: [{ id: 'one', assetPath: 'one.asset', customBuildDir: 'Builds/special' }] }, path.resolve('project/Builds'), path.resolve('project'));
  assert.equal(roots[1].directory, path.resolve('project/Builds/special'));
});

test('public stream drops commands, paths, signing secrets and raw logs', () => {
  assert.equal(publicEvent({ type: 'line', text: 'password private' }), null);
  assert.deepEqual(publicEvent({ type: 'profile-start', name: 'build', index: 0, cmd: 'password', output: 'C:/private' }), { type: 'profile-start', name: 'build', index: 0 });
  assert.equal(publicEvent({ type: 'vcs-line', text: 'password' }), null);
});

test('user API cannot override config and refuses concurrent build', async t => {
  let calls = 0, active = false;
  const api = createUserApi({ config: () => ({ buildNodes: [{ id: 'one', name: 'one', profileAssetPath: 'one.asset', keystorePass: 'private' }], engine: { template: 'secret' } }), job: () => null, busy: () => active, start: async () => { calls++; return { ok: true }; }, stop: () => ({ ok: true }), json: (res, code, data) => { res.writeHead(code, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(data)); }, readBody: async req => { let raw = ''; for await (const chunk of req) raw += chunk; return JSON.parse(raw); } });
  const server = http.createServer((req, res) => api.handle(req, res, new URL(req.url, 'http://localhost')));
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => { api.close(); server.closeAllConnections(); server.close(); });
  const base = `http://127.0.0.1:${server.address().port}`;
  const catalog = await (await fetch(base + '/api/user/catalog')).text();
  assert.ok(!catalog.includes('private') && !catalog.includes('secret') && !catalog.includes('one.asset'));
  const build = body => fetch(base + '/api/user/build', { method: 'POST', body: JSON.stringify(body) });
  assert.equal((await build({ nodeIds: ['one'], engine: { template: 'attack' } })).status, 400);
  assert.equal((await build({ nodeIds: ['missing'] })).status, 400);
  assert.equal((await build({ nodeIds: ['one'] })).status, 200);
  active = true;
  assert.equal((await build({ nodeIds: ['one'] })).status, 409);
  assert.equal(calls, 1);
  assert.equal((await fetch(base + '/api/user/upload', { method: 'POST', body: 'payload' })).status, 404);
});

test('directory ZIP includes all files, unicode names and empty directories', async t => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'tbuild-zip-'));
  t.after(() => fs.rmSync(temp, { recursive: true, force: true }));
  fs.writeFileSync(path.join(temp, '中文.apk'), 'apk-content');
  fs.writeFileSync(path.join(temp, 'build.log'), 'logs');
  fs.mkdirSync(path.join(temp, 'empty'));
  const server = http.createServer((req, res) => folderZip(res, temp, '构建目录').catch(() => res.destroy()));
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => { server.closeAllConnections(); server.close(); });
  const response = await fetch(`http://127.0.0.1:${server.address().port}/`);
  assert.equal(response.status, 200);
  const bytes = Buffer.from(await response.arrayBuffer());
  assert.ok(bytes.includes(Buffer.from('中文.apk')));
  assert.ok(bytes.includes(Buffer.from('apk-content')));
  assert.ok(bytes.includes(Buffer.from('build.log')));
  assert.ok(bytes.includes(Buffer.from('empty/')));
  assert.equal(bytes.readUInt32LE(bytes.length - 22), 0x06054b50);
  assert.equal(bytes.readUInt16LE(bytes.length - 12), 3);
});

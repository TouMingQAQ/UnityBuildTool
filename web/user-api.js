'use strict';
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

function fault(status, message) { const error = new Error(message); error.status = status; return error; }
function within(base, file) { const relative = path.relative(base, file); return relative === '' || (!relative.startsWith('..' + path.sep) && relative !== '..' && !path.isAbsolute(relative)); }

// Reject links/junctions at every level, including links that happen to point inside the root.
async function resolveFile(root, relative = '') {
  if (typeof relative !== 'string' || relative.includes('\0') || relative.includes('\\') || relative.includes(':') || relative.startsWith('/') || relative.split('/').includes('..')) throw fault(403, '只能访问构建目录内的文件');
  const base = path.resolve(root);
  const file = path.resolve(base, relative);
  if (!within(base, file)) throw fault(403, '路径超出构建目录');
  const realBase = await fs.promises.realpath(base);
  let current = base;
  for (const segment of path.relative(base, file).split(path.sep).filter(Boolean)) {
    current = path.join(current, segment);
    if ((await fs.promises.lstat(current)).isSymbolicLink()) throw fault(403, '不支持访问符号链接或目录联接');
  }
  const real = await fs.promises.realpath(file);
  if (!within(realBase, real)) throw fault(403, '路径超出构建目录');
  return { file: real, stat: await fs.promises.stat(real) };
}

function buildNodes(config) {
  const raw = Array.isArray(config.buildNodes) ? config.buildNodes : (config.queue || []).map((node, index) => ({ ...node, ...((config.profileConfigs || {})[node.assetPath] || {}), id: 'legacy-' + index }));
  return raw.filter(node => node && (node.profileAssetPath || node.assetPath)).map(node => ({ ...node, id: String(node.id), assetPath: node.profileAssetPath || node.assetPath }));
}
function buildRequest(config, nodes) {
  return {
    projectPath: config.projectPath, engine: config.engine, outputBase: config.outputBase,
    successDir: config.successDir, artifactNameTemplate: config.artifactNameTemplate,
    autoZip: config.defaultAutoZip !== false, dev: !!config.defaultDevBuild,
    buildAddressables: !!config.defaultBuildAddressables, androidBuildKind: config.defaultAndroidBuildKind || 'apk',
    stopOnError: config.stopOnError !== false, buildNumber: config.buildNumber ?? '-1', proxy: config.proxy,
    profiles: nodes.map(node => ({ ...node, sign: { ...(node.sign || {}), keystoreName: node.keystoreName || '', keyaliasName: node.keyaliasName || '', keystorePass: node.keystorePass || '', keyaliasPass: node.keyaliasPass || '' } })),
  };
}
function outputRoots(config, base, project) {
  if (!config.projectPath && !config.outputBase) return [];
  const roots = [{ id: 'builds', name: '构建目录', directory: base }];
  for (const node of buildNodes(config)) {
    const custom = node.customBuildDir || node.sign?.customBuildDir;
    if (custom) {
      const trimmed = String(custom).trim();
      const norm = trimmed.replace(/^[\\/]+/, '');
      const directory = path.isAbsolute(trimmed) ? path.normalize(trimmed) : /^builds[\\/]/i.test(norm) ? path.join(path.dirname(base), norm) : path.join(base, norm);
      if (!roots.some(root => path.resolve(root.directory) === path.resolve(directory))) roots.push({ id: 'node-' + crypto.createHash('sha256').update(directory).digest('hex').slice(0, 16), name: (node.name || '节点') + ' · 自定义构建目录', directory });
    }
  }
  return roots;
}
function status(job) {
  return job ? { state: job.state, index: job.index, count: job.queue.length, queue: job.queue.map(node => ({ name: node.name, targetName: node.targetName })), progress: job.progress || {}, end: job.end ? { ok: job.end.ok, reason: job.end.reason, message: job.end.message ? '请在管理端查看详细错误日志' : undefined } : null } : null;
}
// A public stream intentionally contains only build lifecycle/progress, never commands,
// signing data, local paths, VCS credentials, or raw Unity output.
function publicEvent(event) {
  const fields = {
    'job-start': ['count'], 'profile-start': ['index', 'name', 'target'],
    progress: ['index', 'stage', 'pct'], 'profile-end': ['index', 'ok', 'skipped'],
    'job-end': ['ok', 'reason'], done: [],
  }[event.type];
  if (!fields) return null;
  const result = { type: event.type };
  for (const field of fields) if (event[field] !== undefined) result[field] = event[field];
  if (event.type === 'done') result.end = event.end ? { ok: event.end.ok, reason: event.end.reason } : null;
  if (event.type === 'job-end' && event.message) result.message = '请在管理端查看详细错误日志';
  return result;
}
function attachment(name) { return "attachment; filename=\"download\"; filename*=UTF-8''" + encodeURIComponent(name).replace(/'/g, '%27'); }
async function sendFile(res, target) {
  res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Length': target.stat.size, 'Content-Disposition': attachment(path.basename(target.file)), 'X-Content-Type-Options': 'nosniff' });
  const stream = fs.createReadStream(target.file);
  res.on('close', () => stream.destroy());
  stream.on('error', () => res.destroy());
  stream.pipe(res);
}

// Stream uncompressed ZIP entries with data descriptors; avoids buffering large builds.
const table = Array.from({ length: 256 }, (_, n) => { for (let k = 0; k < 8; k++) n = n & 1 ? 0xedb88320 ^ (n >>> 1) : n >>> 1; return n >>> 0; });
async function folderZip(res, directory, name) {
  const entries = [];
  async function walk(dir, prefix = '') {
    for (const entry of await fs.promises.readdir(dir, { withFileTypes: true })) {
      if (entry.isSymbolicLink()) continue;
      const relative = prefix + entry.name;
      const target = await resolveFile(directory, relative);
      if (target.stat.isDirectory()) { entries.push({ relative: relative + '/', ...target }); await walk(target.file, relative + '/'); }
      else if (target.stat.isFile()) entries.push({ relative, ...target });
    }
  }
  await walk(directory);
  let estimated = 22;
  for (const entry of entries) estimated += entry.stat.isDirectory() ? 0 : entry.stat.size;
  for (const entry of entries) estimated += 76 + 2 * Buffer.byteLength(entry.relative);
  if (entries.length > 65535 || estimated >= 0xffffffff || entries.some(e => e.stat.size >= 0xffffffff || Buffer.byteLength(e.relative) > 65535)) throw fault(413, '目录超过 ZIP 下载限制（4GB 或 65535 项），请分别下载文件');
  res.writeHead(200, { 'Content-Type': 'application/zip', 'Content-Disposition': attachment(name + '.zip'), 'X-Content-Type-Options': 'nosniff' });
  let offset = 0;
  const central = [];
  async function write(buffer) {
    if (res.destroyed) throw fault(499, '下载已取消');
    offset += buffer.length;
    if (!res.write(buffer)) await new Promise((resolve, reject) => {
      const cleanup = () => { res.off('drain', drain); res.off('close', close); res.off('error', error); };
      const drain = () => { cleanup(); resolve(); };
      const close = () => { cleanup(); reject(fault(499, '下载已取消')); };
      const error = cause => { cleanup(); reject(cause); };
      res.once('drain', drain); res.once('close', close); res.once('error', error);
    });
  }
  for (const entry of entries) {
    const filename = Buffer.from(entry.relative);
    const localOffset = offset;
    const local = Buffer.alloc(30); local.writeUInt32LE(0x04034b50); local.writeUInt16LE(20, 4); local.writeUInt16LE(0x808, 6); local.writeUInt16LE(filename.length, 26);
    await write(local); await write(filename);
    let crc = 0xffffffff, size = 0;
    if (entry.stat.isFile()) {
      const stream = fs.createReadStream(entry.file);
      const close = () => stream.destroy(); res.once('close', close);
      try { for await (const chunk of stream) { size += chunk.length; if (size > entry.stat.size) throw fault(409, '构建文件正在变化，请构建结束后下载'); for (const byte of chunk) crc = table[(crc ^ byte) & 255] ^ (crc >>> 8); await write(chunk); } }
      finally { res.off('close', close); stream.destroy(); }
    }
    crc = (crc ^ 0xffffffff) >>> 0;
    const descriptor = Buffer.alloc(16); descriptor.writeUInt32LE(0x08074b50); descriptor.writeUInt32LE(crc, 4); descriptor.writeUInt32LE(size, 8); descriptor.writeUInt32LE(size, 12); await write(descriptor);
    const header = Buffer.alloc(46); header.writeUInt32LE(0x02014b50); header.writeUInt16LE(20, 4); header.writeUInt16LE(20, 6); header.writeUInt16LE(0x808, 8); header.writeUInt32LE(crc, 16); header.writeUInt32LE(size, 20); header.writeUInt32LE(size, 24); header.writeUInt16LE(filename.length, 28); header.writeUInt32LE(entry.stat.isDirectory() ? 16 : 0, 38); header.writeUInt32LE(localOffset, 42);
    central.push(header, filename);
  }
  const centralOffset = offset;
  for (const buffer of central) await write(buffer);
  const end = Buffer.alloc(22); end.writeUInt32LE(0x06054b50); end.writeUInt16LE(entries.length, 8); end.writeUInt16LE(entries.length, 10); end.writeUInt32LE(offset - centralOffset, 12); end.writeUInt32LE(centralOffset, 16); await write(end); res.end();
}

function createUserApi(context) {
  const clients = new Set();
  let starting = false;
  function broadcast(event) { const safe = publicEvent(event); if (safe) for (const res of clients) res.write('data: ' + JSON.stringify(safe) + '\n\n'); }
  async function handle(req, res, url) {
    if (!url.pathname.startsWith('/api/user/')) return false;
    const { json, readBody } = context;
    const config = context.config();
    const nodes = buildNodes(config);
    const busy = () => starting || context.busy();
    const route = url.pathname;
    try {
      if (req.method === 'GET' && route === '/api/user/catalog') {
        const groups = Array.isArray(config.buildGroups) ? config.buildGroups : [{ id: 'default', name: '默认构建组', nodeIds: nodes.map(n => n.id) }];
        json(res, 200, { nodes: nodes.map(n => ({ id: n.id, name: n.name, targetName: n.targetName, dev: n.dev ?? !!config.defaultDevBuild, bundleVersion: n.bundleVersion })), groups: groups.map(g => ({ id: String(g.id), name: g.name, nodeIds: (g.nodeIds || []).map(String).filter(id => nodes.some(n => n.id === id)) })) });
      } else if (req.method === 'GET' && route === '/api/user/status') json(res, 200, { job: status(context.job()), busy: busy() });
      else if (req.method === 'GET' && route === '/api/user/events') {
        res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache', Connection: 'keep-alive', 'X-Accel-Buffering': 'no' });
        res.write('retry: 3000\n\ndata: ' + JSON.stringify({ type: 'hello', job: status(context.job()) }) + '\n\n');
        clients.add(res); const timer = setInterval(() => res.write(': ping\n\n'), 20000);
        res.on('close', () => { clearInterval(timer); clients.delete(res); });
      } else if (req.method === 'POST' && route === '/api/user/build') {
        const body = await readBody(req);
        if (!body || !Array.isArray(body.nodeIds) || !body.nodeIds.length || Object.keys(body).some(k => k !== 'nodeIds')) throw fault(400, '只能提交已配置的构建节点 ID');
        const selected = [...new Set(body.nodeIds.map(String))].map(id => nodes.find(n => n.id === id));
        if (selected.some(n => !n)) throw fault(400, '构建节点不存在，请刷新列表');
        if (busy()) throw fault(409, '已有构建、检测或仓库更新任务运行中');
        starting = true;
        try { const result = await context.start(buildRequest(config, selected)); if (!result.ok) throw fault(400, '构建配置不可用，请联系管理员检查项目路径和构建引擎'); json(res, 200, { ok: true }); }
        finally { starting = false; }
      } else if (req.method === 'POST' && route === '/api/user/build/stop') json(res, 200, context.stop());
      else if (req.method === 'GET' && ['/api/user/files', '/api/user/download', '/api/user/download-folder'].includes(route)) {
        const roots = outputRoots(config, context.outBase(), context.project());
        const publicRoots = roots.map(({ id, name }) => ({ id, name }));
        if (!roots.length && route === '/api/user/files') { json(res, 200, { roots: [], root: '', path: '', entries: [] }); return true; }
        const root = roots.find(r => r.id === (url.searchParams.get('root') || roots[0]?.id));
        if (!root) throw fault(404, '构建目录不存在');
        const relative = url.searchParams.get('path') || '';
        let target;
        try { target = await resolveFile(root.directory, relative); }
        catch (error) { if (error.code === 'ENOENT' && relative === '' && route === '/api/user/files') { json(res, 200, { roots: publicRoots, root: root.id, path: '', entries: [] }); return true; } throw error; }
        if (route === '/api/user/files') {
          if (!target.stat.isDirectory()) throw fault(400, '请选择文件夹');
          const entries = [];
          for (const entry of await fs.promises.readdir(target.file, { withFileTypes: true })) {
            if (entry.isSymbolicLink()) continue;
            const childPath = relative ? relative.replace(/\/$/, '') + '/' + entry.name : entry.name;
            const child = await resolveFile(root.directory, childPath);
            if (!child.stat.isFile() && !child.stat.isDirectory()) continue;
            entries.push({ name: entry.name, path: childPath, type: child.stat.isDirectory() ? 'directory' : 'file', size: child.stat.isFile() ? child.stat.size : 0, modified: child.stat.mtime.toISOString() });
          }
          entries.sort((a, b) => (a.type === b.type ? a.name.localeCompare(b.name, 'zh-CN') : a.type === 'directory' ? -1 : 1));
          json(res, 200, { roots: publicRoots, root: root.id, path: relative, entries });
        } else if (route === '/api/user/download') { if (!target.stat.isFile()) throw fault(400, '请选择文件'); await sendFile(res, target); }
        else { if (!target.stat.isDirectory()) throw fault(400, '请选择文件夹'); await folderZip(res, target.file, relative ? path.basename(target.file) : root.name); }
      } else json(res, 404, { error: '用户端不提供此操作' });
    } catch (error) {
      if (res.headersSent) res.destroy();
      else json(res, error.status || (error.code === 'ENOENT' ? 404 : 500), { error: error.status ? error.message : error.code === 'ENOENT' ? '文件或文件夹不存在' : '读取失败，请在管理端检查目录权限' });
    }
    return true;
  }
  return { handle, broadcast, close() { for (const res of clients) res.end(); clients.clear(); } };
}
module.exports = { createUserApi, buildNodes, buildRequest, resolveFile, outputRoots, publicEvent, folderZip, within };

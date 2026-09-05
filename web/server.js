'use strict';
/**
 * TBuildTool · 外部打包工具（本地 Web 批量打包器）—— 后端（零依赖 Node.js，Node 18+）
 *
 * 功能：
 *   - 配置 Unity 项目路径（绝对 / 相对路径，相对路径以本工具目录为基准）
 *   - 扫描项目里的 Build Profile（Unity 6），支持 Win / Android / iOS / macOS
 *   - 选择 Profile 并排序，支持每个节点自定义缓存目录、命名规则、Addressables 与 DEV
 *   - 按顺序逐个调用命令行打包（Unity.exe -batchmode 或自定义 CLI 模板）
 *   - SSE 实时推送每步日志，支持停止、失败即停等策略
 *
 * 启动：node server.js   （自动打开浏览器 http://127.0.0.1:8787）
 * 环境变量：PORT 端口（默认 8787）；NO_OPEN=1 时不自动开浏览器
 */

const http = require('http');
const fs = require('fs');
const path = require('path');
const { spawn, spawnSync } = require('child_process');
const zlib = require('zlib');

const ROOT = __dirname;
const PUBLIC = path.join(ROOT, 'public');
const CONFIG_PATH = path.join(ROOT, 'config.json');
const PORT = Number(process.env.PORT || 8787);
const HOST = '127.0.0.1';
const AUTO_OPEN = process.env.NO_OPEN !== '1';

// BuildTarget 枚举中我们关心的目标：Android(13)、Win64(19)、Win86(5)、iOS(9)、macOS(4)
const TARGETS = { 5: 'Windows (x86)', 19: 'Windows (x64)', 13: 'Android', 9: 'iOS', 4: 'macOS' };
const ALLOWED_TARGETS = new Set([5, 13, 19, 9, 4]);
// 环境编译检测结果目录（相对本工具目录）
const CHECK_RESULTS_DIR = path.join(__dirname, 'check-results');

// ─────────────────────────── 配置 ───────────────────────────

function loadConfig() {
  let cfg;
  try {
    let txt = fs.readFileSync(CONFIG_PATH, 'utf8');
    txt = txt.replace(/^\uFEFF/, '');
    cfg = JSON.parse(txt);
  } catch { cfg = {}; }
  if (cfg && typeof cfg === 'object') delete cfg.outputs;
  return cfg;
}
let config = loadConfig();

function saveConfig() {
  try { fs.writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2)); } catch (e) {
    console.error('[config] 保存失败:', e.message);
  }
}

function cfg(key, def) {
  const v = config[key];
  return v === undefined || v === null || v === '' ? def : v;
}

// ─────────────────────────── 工具函数 ───────────────────────────

function resolveProject(p) {
  if (!p || !String(p).trim()) return null;
  const abs = path.isAbsolute(p) ? p : path.resolve(ROOT, p);
  return path.normalize(abs);
}

function fwd(p) { return p.split(path.sep).join('/'); }

function readUnityVersion(projectAbs) {
  try {
    const txt = fs.readFileSync(path.join(projectAbs, 'ProjectSettings', 'ProjectVersion.txt'), 'utf8');
    const m = txt.match(/m_EditorVersion:\s*(\S+)/);
    return m ? m[1] : null;
  } catch { return null; }
}

function isValidProject(projectAbs) {
  return projectAbs &&
    fs.existsSync(path.join(projectAbs, 'ProjectSettings')) &&
    fs.existsSync(path.join(projectAbs, 'Assets'));
}

function cmpVer(a, b) {
  const pa = a.split(/[^\d]+/).map(Number), pb = b.split(/[^\d]+/).map(Number);
  for (let i = 0; i < 3; i++) {
    const d = (pb[i] || 0) - (pa[i] || 0);
    if (d) return d;
  }
  return 0;
}

// ─────────────────────────── Profile 扫描 ───────────────────────────

function scanProfiles(projectAbs, profileDir) {
  const dirs = [];
  const custom = (profileDir || '').trim();
  if (custom) dirs.push(path.isAbsolute(custom) ? custom : path.join(projectAbs, custom));
  dirs.push(path.join(projectAbs, 'Assets', 'Settings', 'Build Profiles')); // Unity 6 默认位置

  const seen = new Set();
  const found = [], ignored = [];
  for (const d of dirs) {
    if (!fs.existsSync(d)) continue;
    let files = [];
    try { files = fs.readdirSync(d).filter(f => f.endsWith('.asset')); } catch { continue; }
    for (const f of files) {
      const abs = path.join(d, f);
      if (seen.has(abs)) continue;
      seen.add(abs);
      const p = parseProfile(projectAbs, abs);
      if (!p) continue;
      if (ALLOWED_TARGETS.has(p.target)) found.push(p);
      else ignored.push({ name: p.name, targetName: p.targetName });
    }
  }
  found.sort((a, b) => a.name.localeCompare(b.name));
  return { found, ignored };
}

function extractYamlQuoted(text, key) {
  const line = text.split('\n').find(l => l.includes(key + ':'));
  if (!line) return null;
  const idx = line.indexOf(key + ':');
  return line.substring(idx + key.length + 1).trim().replace(/^'+|'+$/g, '');
}

function parseProfile(projectAbs, abs) {
  let txt;
  try { txt = fs.readFileSync(abs, 'utf8'); } catch { return null; }
  if (!txt.includes('BuildProfile')) return null;

  const name = txt.match(/^\s{2}m_Name:\s*(.+)$/m);
  const tgt = txt.match(/^\s{2}m_BuildTarget:\s*(\d+)\s*$/m);
  const sub = txt.match(/^\s{2}m_Subtarget:\s*(\d+)\s*$/m);
  const scenes = (txt.match(/^\s{4}m_path:\s*/gm) || []).length;
  const prod = txt.match(/productName:\s*([^\r\n']+)/);
  const ksName = extractYamlQuoted(txt, 'AndroidKeystoreName');
  const ksAlias = extractYamlQuoted(txt, 'AndroidKeyaliasName');
  const ksCustom = txt.match(/androidUseCustomKeystore:\s*(\d+)/);
  const bundleVer = extractYamlQuoted(txt, 'bundleVersion');
  if (!tgt) return null;

  const target = Number(tgt[1]);
  return {
    name: name ? name[1].trim() : path.basename(abs, '.asset'),
    target,
    targetName: TARGETS[target] || ('BuildTarget ' + target),
    subtarget: sub ? Number(sub[1]) : 0,
    sceneCount: scenes,
    productName: prod ? prod[1].trim() : null,
    bundleVersion: bundleVer ? bundleVer.trim() : null,
    keystoreName: ksName ? ksName.trim() : null,
    keyaliasName: ksAlias ? ksAlias.trim() : null,
    useCustomKeystore: ksCustom ? Number(ksCustom[1]) === 1 : false,
    absPath: abs,
    assetPath: fwd(path.relative(projectAbs, abs)),
  };
}

// ─────────────────────────── 编辑器 / CLI 探测 ───────────────────────────

function findUnityExes() {
  const roots = [];
  if (process.env.ProgramFiles) roots.push(path.join(process.env.ProgramFiles, 'Unity', 'Hub', 'Editor'));
  if (process.env['ProgramFiles(x86)']) roots.push(path.join(process.env['ProgramFiles(x86)'], 'Unity', 'Hub', 'Editor'));
  try {
    const f = path.join(process.env.APPDATA || '', 'UnityHub', 'secondaryInstallPath.json');
    if (fs.existsSync(f)) {
      const raw = JSON.parse(fs.readFileSync(f, 'utf8'));
      const list = Array.isArray(raw) ? raw : [raw];
      for (const it of list) {
        const s = typeof it === 'string' ? it : (it.path || '');
        if (s) roots.push(s);
      }
    }
  } catch { /* 忽略 */ }

  const exes = [];
  for (const root of roots) {
    if (!fs.existsSync(root)) continue;
    let entries = [];
    try { entries = fs.readdirSync(root); } catch { continue; }
    for (const d of entries) {
      if (!/^\d+\.\d+\.\d+/.test(d)) continue;
      const exe = path.join(root, d, 'Editor', 'Unity.exe');
      if (fs.existsSync(exe)) exes.push({ version: d, path: exe });
    }
  }
  exes.sort((a, b) => cmpVer(a.version, b.version));
  return exes;
}

function findUnityCli() {
  const candidates = [];
  if (process.env.LOCALAPPDATA) candidates.push(path.join(process.env.LOCALAPPDATA, 'Unity', 'bin', 'unity.exe'));
  try {
    const r = spawnSync('unity', ['--version'], { encoding: 'utf8', timeout: 8000, windowsHide: true });
    if (r.status === 0) candidates.unshift('unity');
  } catch { /* PATH 里没有 */ }
  for (const c of candidates) {
    if (fs.existsSync(c) || c === 'unity') return c;
  }
  return null;
}

function probeWithUnityCli() {
  const cli = findUnityCli();
  if (!cli) return { cli: null, exes: [] };
  try {
    const r = spawnSync(cli, ['--format', 'json', 'editors', 'list'], {
      encoding: 'utf8', timeout: 15000, windowsHide: true,
      env: { ...process.env, UNITY_CLI_NO_LOCK: '1' },
    });
    const text = (r.stdout || '') + '\n' + (r.stderr || '');
    const exes = [];
    const re = /([A-Za-z]:[^\r\n"']*?Editor[\\/]Unity\.exe)/g;
    let m;
    while ((m = re.exec(text)) !== null) {
      const p = m[1].replace(/\\/g, '\\');
      const verMatch = p.match(/(\d+\.\d+\.\d+\w*)[\\/]Editor[\\/]Unity\.exe$/);
      exes.push({ version: verMatch ? verMatch[1] : '?', path: p });
    }
    const seen = new Set();
    const uniq = exes.filter(e => { if (seen.has(e.path)) return false; seen.add(e.path); return true; });
    uniq.sort((a, b) => cmpVer(a.version, b.version));
    return { cli, exes: uniq, raw: text.slice(0, 2000) };
  } catch (e) {
    return { cli, exes: [], error: e.message };
  }
}

// ─────────────────────────── 命令与产物逻辑 ───────────────────────────

function quote(s) { return '"' + String(s).replace(/"/g, '\\"') + '"'; }

function parseCmd(s) {
  const out = []; let cur = '', q = false;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === '"') q = !q;
    else if (c === ' ' && !q) { if (cur) { out.push(cur); cur = ''; } }
    else cur += c;
  }
  if (cur) out.push(cur);
  return out;
}

/**
 * 需求 1：支持单独指定构建目标文件夹，可多 Profile 公用缓存目录
 */
function makeOutputs(profile, outBase, customBuildDir, androidBuildKind) {
  let dir;
  if (customBuildDir && String(customBuildDir).trim()) {
    const trimmed = String(customBuildDir).trim();
    if (path.isAbsolute(trimmed)) {
      dir = path.normalize(trimmed);
    } else {
      const norm = trimmed.replace(/^[\\/]+/, '');
      if (norm.toLowerCase().startsWith('builds' + path.sep.toLowerCase()) || norm.toLowerCase().startsWith('builds/')) {
        dir = path.join(path.dirname(outBase), norm);
      } else {
        dir = path.join(outBase, norm);
      }
    }
  } else {
    dir = path.join(outBase, profile.name);
  }
  const prod = safeName(profile.productName || profile.name);
  let output = '';
  if (profile.target === 13) {
    const bkind = String(profile.androidBuildKind || androidBuildKind || 'apk').toLowerCase();
    if (bkind === 'aab') {
      output = path.join(dir, prod + '.aab');
    } else if (bkind === 'gradleproject' || bkind === 'gradle' || bkind === 'asproject') {
      output = path.join(dir, prod + '-ASProject'); // 导出 Android Studio 工程目录
    } else {
      output = path.join(dir, prod + '.apk');
    }
  } else if (profile.target === 9) {
    output = path.join(dir, 'iOS'); // Xcode 工程输出目录
  } else if (profile.target === 4) {
    output = path.join(dir, prod + '.app');
  } else {
    output = path.join(dir, prod + '.exe');
  }
  return { dir, output, logFile: path.join(dir, 'build.log'), isSharedDir: !!(customBuildDir && String(customBuildDir).trim()) };
}

function pad2(n) { return String(n).padStart(2, '0'); }
function formatNameTs(d) {
  return `${d.getFullYear()}${pad2(d.getMonth() + 1)}${pad2(d.getDate())}_${pad2(d.getHours())}${pad2(d.getMinutes())}${pad2(d.getSeconds())}`;
}
function formatDateTime(d) {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
}
function safeName(s) { return String(s).replace(/[\\/:*?"<>|\r\n]/g, '_').trim() || 'unknown'; }

/**
 * 需求 2：自定义构建成品命名规则（占位符替换）
 */
function formatArtifactName(tmpl, meta) {
  let s = (tmpl || '{Product}_{Platform}_v{Version}_b{VersionCode}_{Time}{Dev}').trim();
  const map = {
    '{Platform}': meta.Platform || meta.Target || '',
    '{Target}': meta.Platform || meta.Target || '',
    '{Profile}': meta.Profile || '',
    '{ProfileName}': meta.Profile || '',
    '{Product}': meta.Product || meta.Profile || 'App',
    '{ProductName}': meta.Product || meta.Profile || 'App',
    '{Version}': meta.Version || '',
    '{VersionCode}': meta.VersionCode != null ? String(meta.VersionCode) : '',
    '{BuildNumber}': meta.VersionCode != null ? String(meta.VersionCode) : '',
    '{Time}': meta.Time || '',
    '{DateTime}': meta.Time || '',
    '{Date}': meta.Date || '',
    '{Dev}': meta.Dev ? '_DEV' : '',
    '{DEV}': meta.Dev ? 'DEV' : '',
  };
  for (const k in map) {
    s = s.split(k).join(map[k]);
  }
  s = s.replace(/_{2,}/g, '_').replace(/-{2,}/g, '-').replace(/^[-_]+|[-_]+$/g, '');
  return safeName(s) || 'artifact';
}

// ── 零依赖 ZIP 打包与严格文件过滤 ──

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = (c & 1) ? (0xEDB88320 ^ (c >>> 1)) : (c >>> 1);
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xFFFFFFFF;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xFF] ^ (c >>> 8);
  return (c ^ 0xFFFFFFFF) >>> 0;
}

function dosDateTime(d) {
  const time = (d.getHours() << 11) | (d.getMinutes() << 5) | (d.getSeconds() >> 1);
  const date = ((d.getFullYear() - 1980) << 9) | ((d.getMonth() + 1) << 5) | d.getDate();
  return { time: time & 0xFFFF, date: date & 0xFFFF };
}

/**
 * 严格过滤：禁止将 Unity 调试符号备份、编译临时缓存、日志和系统残留打包进 ZIP 产物
 */
function shouldExcludeFromZip(relPath, fileName, isDirectory, target) {
  const lowerName = fileName.toLowerCase();
  const lowerRel = relPath.replace(/\\/g, '/').toLowerCase();

  // 1. 系统残留与版本控制元数据
  if (lowerName === '.ds_store' || lowerName === 'thumbs.db' || lowerName === 'desktop.ini' || lowerName === 'ehthumbs.db') return true;
  if (lowerName === '.git' || lowerName === '.svn' || lowerName === '.hg' || lowerName === '.vs' || lowerName === '.idea' || lowerName === '.vscode') return true;

  // 2. 日志与崩溃转储文件
  if (lowerName === 'build.log' || lowerName.endsWith('.log') || lowerName.endsWith('.dmp') || lowerName.endsWith('.mdmp')) return true;

  // 3. Unity 调试与符号备份文件夹（Unity 官方明确标记 DoNotShip / DontShip）
  if (lowerName.includes('burstdebuginformation_donotship') ||
      lowerName.includes('backupthisfolder_butdontshipitwithyourgame') ||
      lowerName.includes('backupthisfolder') ||
      lowerName.includes('donotship') ||
      lowerName.includes('dontship')) {
    return true;
  }

  // 4. 临时文件与编译中间文件
  if (lowerName.endsWith('.tmp') || lowerName.endsWith('.temp') || lowerName.endsWith('.bak') || lowerName.endsWith('.swp') || lowerName.startsWith('~')) return true;
  if (lowerName.endsWith('.ilk') || lowerName.endsWith('.exp') || lowerName.endsWith('.tlog') || lowerName.endsWith('.idb') || lowerName.endsWith('.lastcodeanalysissucceeded')) return true;

  // 5. 避免将其它��品打包进当前 zip
  if (!isDirectory && (lowerName.endsWith('.apk') || lowerName.endsWith('.aab') || lowerName.endsWith('.zip'))) {
    return true;
  }

  // 6. iOS Xcode 工程特定排除：避免打包 Xcode 临时 DerivedData 或本地 build 缓存
  if (target === 9 || lowerRel.includes('xcode') || lowerRel.includes('ios')) {
    if (isDirectory && (lowerName === 'deriveddata' || lowerName === 'build' || lowerName.endsWith('.intermediates'))) return true;
    if (lowerRel.startsWith('deriveddata/') || lowerRel.startsWith('build/') || lowerRel.includes('/deriveddata/') || lowerRel.includes('/build/')) return true;
    if (lowerName.endsWith('.dsym')) return true;
  }

  return false;
}

function collectZipFiles(srcDir, target) {
  const files = [];
  (function walk(dir) {
    let entries;
    try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
    for (const ent of entries) {
      const full = path.join(dir, ent.name);
      const rel = path.relative(srcDir, full);
      if (shouldExcludeFromZip(rel, ent.name, ent.isDirectory(), target)) {
        continue;
      }
      if (ent.isDirectory()) {
        walk(full);
      } else if (ent.isFile()) {
        files.push(full);
      }
    }
  })(srcDir);
  return files.sort();
}

function writeZip(srcDir, zipPath, target) {
  const files = collectZipFiles(srcDir, target);
  if (!files.length) throw new Error('构建目录为空或所有文件均被过滤，无法生成 zip: ' + srcDir);
  const fd = fs.openSync(zipPath, 'w');
  const central = [];
  let offset = 0;
  const now = dosDateTime(new Date());
  try {
    for (const f of files) {
      const buf = fs.readFileSync(f);
      if (buf.length > 0xFFFFFFFF) throw new Error('文件超过 4GB，超出标准 ZIP 上限: ' + f);
      const name = path.relative(srcDir, f).split(path.sep).join('/');
      const nameBuf = Buffer.from(name, 'utf8');
      const comp = zlib.deflateRawSync(buf, { level: 6 });
      const crc = crc32(buf);
      const lh = Buffer.alloc(30);
      lh.writeUInt32LE(0x04034b50, 0);
      lh.writeUInt16LE(20, 4);
      lh.writeUInt16LE(0x0800, 6);
      lh.writeUInt16LE(8, 8);
      lh.writeUInt16LE(now.time, 10);
      lh.writeUInt16LE(now.date, 12);
      lh.writeUInt32LE(crc, 14);
      lh.writeUInt32LE(comp.length, 18);
      lh.writeUInt32LE(buf.length, 22);
      lh.writeUInt16LE(nameBuf.length, 26);
      lh.writeUInt16LE(0, 28);
      fs.writeSync(fd, lh);
      fs.writeSync(fd, nameBuf);
      fs.writeSync(fd, comp);
      central.push({ nameBuf, crc, compLen: comp.length, uncompLen: buf.length, offset });
      offset += 30 + nameBuf.length + comp.length;
    }
    const cdStart = offset;
    for (const e of central) {
      const cd = Buffer.alloc(46);
      cd.writeUInt32LE(0x02014b50, 0);
      cd.writeUInt16LE(20, 4);
      cd.writeUInt16LE(20, 6);
      cd.writeUInt16LE(0x0800, 8);
      cd.writeUInt16LE(8, 10);
      cd.writeUInt16LE(now.time, 12);
      cd.writeUInt16LE(now.date, 14);
      cd.writeUInt32LE(e.crc, 16);
      cd.writeUInt32LE(e.compLen, 20);
      cd.writeUInt32LE(e.uncompLen, 24);
      cd.writeUInt16LE(e.nameBuf.length, 28);
      cd.writeUInt16LE(0, 30);
      cd.writeUInt16LE(0, 32);
      cd.writeUInt16LE(0, 34);
      cd.writeUInt16LE(0, 36);
      cd.writeUInt32LE(0, 38);
      cd.writeUInt32LE(e.offset, 42);
      fs.writeSync(fd, cd);
      fs.writeSync(fd, e.nameBuf);
    }
    const cdSize = central.reduce((s, e) => s + 46 + e.nameBuf.length, 0);
    const eocd = Buffer.alloc(22);
    eocd.writeUInt32LE(0x06054b50, 0);
    eocd.writeUInt16LE(0, 4);
    eocd.writeUInt16LE(0, 6);
    eocd.writeUInt16LE(central.length & 0xFFFF, 8);
    eocd.writeUInt16LE(central.length & 0xFFFF, 10);
    eocd.writeUInt32LE(cdSize, 12);
    eocd.writeUInt32LE(cdStart, 16);
    eocd.writeUInt16LE(0, 20);
    fs.writeSync(fd, eocd);
  } finally {
    fs.closeSync(fd);
  }
}

function findArtifact(dir, re) {
  try {
    const files = fs.readdirSync(dir).filter(f => re.test(f));
    files.sort((a, b) => {
      try { return fs.statSync(path.join(dir, b)).mtimeMs - fs.statSync(path.join(dir, a)).mtimeMs; } catch { return 0; }
    });
    return files.length ? path.join(dir, files[0]) : null;
  } catch { return null; }
}

function readBuildInfo(dir) {
  try {
    const txt = fs.readFileSync(path.join(dir, 'build-info.txt'), 'utf8');
    const o = {};
    for (const line of txt.split(/\r?\n/)) {
      const m = line.match(/^([^:\r\n]+):\s*(.*)$/);
      if (m) o[m[1].trim().toLowerCase()] = m[2].trim();
    }
    return o;
  } catch { return null; }
}

function resolveOutBase() {
  const projectAbs = resolveProject(cfg('projectPath', '../..'));
  const ob = cfg('outputBase', 'Builds');
  return ob ? (path.isAbsolute(ob) ? path.normalize(ob) : path.join(projectAbs || '', ob)) : path.join(projectAbs || '', 'Builds');
}

function scanOutputs(outBase) {
  const items = [];
  if (!outBase || !fs.existsSync(outBase)) return items;
  const successName = cfg('successDir', '构建成功');
  const successAbs = path.join(outBase, successName);
  const exts = /\.(apk|aab|zip|exe)$/i;
  const seen = new Set();

  if (fs.existsSync(successAbs)) {
    let subs = [];
    try { subs = fs.readdirSync(successAbs); } catch { subs = []; }
    for (const d of subs) {
      const dir = path.join(successAbs, d);
      let st; try { st = fs.statSync(dir); } catch { continue; }
      if (!st.isDirectory()) continue;
      let files = [];
      try { files = fs.readdirSync(dir).filter(f => exts.test(f)); } catch { continue; }
      for (const f of files) {
        const file = path.join(dir, f);
        let fst; try { fst = fs.statSync(file); } catch { continue; }
        const info = readBuildInfo(dir) || {};
        const isApk = f.toLowerCase().endsWith('.apk') || f.toLowerCase().endsWith('.aab');
        const isExe = f.toLowerCase().endsWith('.exe');
        const isIOS = (info.target || '').toLowerCase().includes('ios');
        items.push({
          name: info.profile || d.replace(/-\d{8}_\d{6}$/, '') || d,
          targetName: info.target || (isApk ? 'Android' : isIOS ? 'iOS' : isExe ? 'Windows' : ''),
          file, dir, time: fst.mtimeMs,
          version: info.version || null,
          versionCode: info.buildnumber !== undefined && info.buildnumber !== '' ? (Number(info.buildnumber) || info.buildnumber) : null,
          date: info.date || null,
          kind: isApk ? 'apk' : isIOS ? 'ios' : isExe ? 'exe' : 'zip',
          size: fst.size,
          dev: info.dev === 'true',
        });
        seen.add(file);
      }
    }
  }

  // 同时也扫描非压缩直接输出的目录（供直接运行查看）
  let directDirs = [];
  try { directDirs = fs.readdirSync(outBase); } catch { directDirs = []; }
  for (const d of directDirs) {
    if (d === successName) continue;
    const dir = path.join(outBase, d);
    let st; try { st = fs.statSync(dir); } catch { continue; }
    if (!st.isDirectory()) continue;
    let files = [];
    try { files = fs.readdirSync(dir).filter(f => exts.test(f)); } catch { continue; }
    for (const f of files) {
      const file = path.join(dir, f);
      if (seen.has(file)) continue;
      let fst; try { fst = fs.statSync(file); } catch { continue; }
      const info = readBuildInfo(dir) || {};
      const isApk = f.toLowerCase().endsWith('.apk') || f.toLowerCase().endsWith('.aab');
      const isExe = f.toLowerCase().endsWith('.exe');
      const isIOS = (info.target || '').toLowerCase().includes('ios');
      items.push({
        name: info.profile || d,
        targetName: info.target || (isApk ? 'Android' : isIOS ? 'iOS' : isExe ? 'Windows' : ''),
        file, dir, time: fst.mtimeMs,
        version: info.version || null,
        versionCode: info.buildnumber !== undefined && info.buildnumber !== '' ? (Number(info.buildnumber) || info.buildnumber) : null,
        date: info.date || null,
        kind: isApk ? 'apk' : isIOS ? 'ios' : isExe ? 'exe' : 'zip',
        size: fst.size,
        dev: info.dev === 'true',
      });
      seen.add(file);
    }
  }

  items.sort((a, b) => b.time - a.time);
  return items.slice(0, 50);
}

/**
 * 产物归档、重命名、选择性 ZIP 压缩（支持保留未压缩原文件直接运行）
 */
function finalizeArtifact(profile, ctx, durationMs) {
  const successName = ctx.successDir || cfg('successDir', '构建成功');
  const successBase = path.join(ctx.outBase, successName);
  const now = new Date();
  const tsStr = formatNameTs(now);
  const dateStr = formatDateTime(now);
  const version = (profile.bundleVersion || '').trim();
  const prod = safeName(profile.productName || profile.name);
  const isAndroid = profile.target === 13;
  const isIOS = profile.target === 9;
  const isMac = profile.target === 4;
  const buildNo = ctx.buildNo;
  const autoZip = ctx.autoZip !== false;

  // 格式化产物命名
  const formattedBaseName = formatArtifactName(ctx.nameTemplate || cfg('artifactNameTemplate', ''), {
    Platform: profile.targetName || (isAndroid ? 'Android' : isIOS ? 'iOS' : 'Windows'),
    Target: profile.targetName || (isAndroid ? 'Android' : isIOS ? 'iOS' : 'Windows'),
    Profile: profile.name,
    ProfileName: profile.name,
    Product: prod,
    ProductName: prod,
    Version: version,
    VersionCode: buildNo,
    BuildNumber: buildNo,
    Time: tsStr,
    Date: tsStr.slice(0, 8),
    Dev: !!ctx.dev,
  });

  const finalDir = path.join(successBase, safeName(profile.name) + '-' + tsStr);
  fs.mkdirSync(finalDir, { recursive: true });

  let finalFile = null, sizeBytes = 0, kind = 'zip';
  if (isAndroid) {
    const bkind = String(ctx.androidBuildKind || 'apk').toLowerCase();
    if (bkind === 'gradleproject' || bkind === 'gradle' || bkind === 'asproject') {
      // 导出 Android Studio 工程：输出为目录，按 Xcode 工程方式归档（自动 ZIP）
      let projDir = ctx.output;
      if (!fs.existsSync(projDir) || !fs.statSync(projDir).isDirectory()) {
        projDir = path.join(ctx.dir, prod + '-ASProject');
        if (!fs.existsSync(projDir)) projDir = ctx.dir;
      }
      fs.writeFileSync(path.join(projDir, 'build-info.txt'),
        ['Profile: ' + profile.name, 'Target: Android (Gradle Project)', 'Product: ' + prod,
         'Version: ' + (version || ''), 'BuildNumber: ' + (buildNo != null ? buildNo : ''),
         'DEV: ' + (ctx.dev ? 'true' : 'false'), 'Date: ' + dateStr].join('\n') + '\n', 'utf8');
      if (autoZip) {
        kind = 'zip';
        finalFile = path.join(finalDir, formattedBaseName + '.zip');
        writeZip(projDir, finalFile, profile.target);
        sizeBytes = fs.statSync(finalFile).size;
      } else {
        kind = 'asproj';
        finalFile = projDir;
        try {
          sizeBytes = collectZipFiles(projDir, profile.target).reduce((acc, f) => {
            try { return acc + fs.statSync(f).size; } catch { return acc; }
          }, 0);
        } catch { sizeBytes = 0; }
      }
    } else {
      kind = 'apk';
      const src = findArtifact(ctx.dir, /\.(apk|aab)$/i);
      if (!src) throw new Error('构建目录中找不到 APK/AAB 产物: ' + ctx.dir);
      const ext = path.extname(src) || '.apk';
      finalFile = path.join(finalDir, formattedBaseName + ext);
      fs.renameSync(src, finalFile);
      sizeBytes = fs.statSync(finalFile).size;
    }
  } else if (isIOS) {
    let xcodeDir = ctx.output;
    if (!fs.existsSync(xcodeDir) || !fs.statSync(xcodeDir).isDirectory()) {
      xcodeDir = path.join(ctx.dir, 'iOS');
      if (!fs.existsSync(xcodeDir)) xcodeDir = ctx.dir;
    }
    fs.writeFileSync(path.join(xcodeDir, 'build-info.txt'),
      ['Profile: ' + profile.name, 'Target: iOS (Xcode)', 'Product: ' + prod,
       'Version: ' + (version || ''), 'BuildNumber: ' + (buildNo != null ? buildNo : ''),
       'DEV: ' + (ctx.dev ? 'true' : 'false'), 'Date: ' + dateStr].join('\n') + '\n', 'utf8');

    if (autoZip) {
      kind = 'ios';
      finalFile = path.join(finalDir, formattedBaseName + '.zip');
      writeZip(xcodeDir, finalFile, profile.target);
      sizeBytes = fs.statSync(finalFile).size;
    } else {
      kind = 'ios';
      finalFile = xcodeDir;
      try {
        sizeBytes = collectZipFiles(xcodeDir, profile.target).reduce((acc, f) => {
          try { return acc + fs.statSync(f).size; } catch { return acc; }
        }, 0);
      } catch { sizeBytes = 0; }
    }
  } else if (isMac) {
    fs.writeFileSync(path.join(ctx.dir, 'build-info.txt'),
      ['Profile: ' + profile.name, 'Target: macOS', 'Product: ' + prod,
       'Version: ' + (version || ''), 'BuildNumber: ' + (buildNo != null ? buildNo : ''),
       'DEV: ' + (ctx.dev ? 'true' : 'false'), 'Date: ' + dateStr].join('\n') + '\n', 'utf8');

    if (autoZip) {
      kind = 'zip';
      finalFile = path.join(finalDir, formattedBaseName + '.zip');
      writeZip(ctx.dir, finalFile, profile.target);
      sizeBytes = fs.statSync(finalFile).size;
    } else {
      kind = 'zip';
      finalFile = ctx.output || ctx.dir;
      try {
        sizeBytes = collectZipFiles(ctx.dir, profile.target).reduce((acc, f) => {
          try { return acc + fs.statSync(f).size; } catch { return acc; }
        }, 0);
      } catch { sizeBytes = 0; }
    }
  } else {
    // Windows
    fs.mkdirSync(ctx.dir, { recursive: true });
    fs.writeFileSync(path.join(ctx.dir, 'build-info.txt'),
      ['Profile: ' + profile.name, 'Target: ' + (profile.targetName || 'Windows'), 'Product: ' + prod,
       'Version: ' + (version || ''), 'BuildNumber: ' + (buildNo != null ? buildNo : ''),
       'DEV: ' + (ctx.dev ? 'true' : 'false'), 'Date: ' + dateStr].join('\n') + '\n', 'utf8');

    if (autoZip) {
      kind = 'zip';
      finalFile = path.join(finalDir, formattedBaseName + '.zip');
      writeZip(ctx.dir, finalFile, profile.target);
      sizeBytes = fs.statSync(finalFile).size;
    } else {
      kind = 'exe';
      const exeFile = path.join(ctx.dir, prod + '.exe');
      finalFile = fs.existsSync(exeFile) ? exeFile : ctx.output;
      try {
        sizeBytes = fs.existsSync(finalFile) ? fs.statSync(finalFile).size : 0;
      } catch { sizeBytes = 0; }
    }
  }

  const info = [
    'Profile: ' + profile.name,
    'Target: ' + (profile.targetName || (isAndroid ? 'Android' : isIOS ? 'iOS' : 'Windows')),
    'Product: ' + prod,
    'Version: ' + (version || ''),
    'BuildNumber: ' + (buildNo != null ? buildNo : ''),
    'DEV: ' + (ctx.dev ? 'true' : 'false'),
    'Date: ' + dateStr,
    'Output: ' + path.basename(finalFile),
    'Size: ' + sizeBytes,
    'Duration: ' + Math.round(durationMs / 1000) + 's',
    'SourceDir: ' + ctx.dir,
  ];
  fs.writeFileSync(path.join(finalDir, 'build-info.txt'), info.join('\n') + '\n', 'utf8');

  return {
    name: profile.name,
    targetName: profile.targetName || (isAndroid ? 'Android' : isIOS ? 'iOS' : 'Windows'),
    file: finalFile, dir: finalDir, time: now.getTime(),
    version: version || null,
    versionCode: buildNo != null ? buildNo : null,
    date: dateStr,
    kind,
    size: sizeBytes,
    dev: !!ctx.dev,
  };
}

function composeBatchmode(eng, ctx) {
  const args = ['-batchmode'];
  if (eng.nographics !== false) args.push('-nographics');
  args.push('-quit', '-projectPath', ctx.projectAbs,
    '-executeMethod', 'TBuildTool.Editor.BuildCommand.Build',
    '-profilePath', ctx.profile.assetPath,
    '-outputPath', ctx.output,
    '-wallpaperTarget', String(ctx.profile.target),
    '-logFile', ctx.logFile);

  // 需求 5：DEV 选项
  if (ctx.dev) {
    args.push('-dev', 'true');
  }

  // 需求 3：Addressables 选项
  if (ctx.buildAddressables) {
    args.push('-buildAddressables', ctx.addressablesMethod ? ctx.addressablesMethod : 'true');
  }

  // 需求 4：安卓签名参数（仅安卓目标）
  if (ctx.profile.target === 13 && ctx.sign) {
    if (ctx.sign.keystoreName) args.push('-keystoreName', ctx.sign.keystoreName);
    if (ctx.sign.keyaliasName) args.push('-keystoreAlias', ctx.sign.keyaliasName);
    if (ctx.sign.keystorePass) args.push('-keystorePass', ctx.sign.keystorePass);
    if (ctx.sign.keyaliasPass) args.push('-keyaliasPass', ctx.sign.keyaliasPass);
  }
  // 安卓构建号（bundleVersionCode）
  if (ctx.profile.target === 13 && ctx.versionCode) args.push('-versionCode', String(ctx.versionCode));
  // 安卓构建目标类型（apk / aab / gradleProject 导出 AS 工程）
  if (ctx.profile.target === 13 && ctx.androidBuildKind && String(ctx.androidBuildKind).toLowerCase() !== 'apk') {
    args.push('-androidBuildKind', String(ctx.androidBuildKind).toLowerCase());
  }
  return { exe: eng.unityExe, args, display: [quote(eng.unityExe)].concat(args.map(quote)).join(' ') };
}

function composeTemplate(eng, ctx) {
  let t = (eng.template || '').trim();
  const map = {
    '{cli}': eng.cliPath || 'unity',
    '{project}': ctx.projectAbs,
    '{profilePath}': ctx.profile.assetPath,
    '{profileAbs}': ctx.profile.absPath,
    '{profileName}': ctx.profile.name,
    '{target}': ctx.profile.targetName,
    '{output}': ctx.output,
    '{logFile}': ctx.logFile,
    '{dev}': ctx.dev ? '-dev true' : '',
    '{buildAddressables}': ctx.buildAddressables ? ('-buildAddressables ' + (ctx.addressablesMethod || 'true')) : '',
    '{keystorePass}': ctx.sign && ctx.sign.keystorePass || '',
    '{keyaliasPass}': ctx.sign && ctx.sign.keyaliasPass || '',
    '{keystoreName}': ctx.sign && ctx.sign.keystoreName || '',
    '{keyaliasName}': ctx.sign && ctx.sign.keyaliasName || '',
    '{androidBuildKind}': (ctx.profile.target === 13 && ctx.androidBuildKind && String(ctx.androidBuildKind).toLowerCase() !== 'apk') ? ('-androidBuildKind ' + String(ctx.androidBuildKind).toLowerCase()) : '',
    '{unityExe}': eng.unityExe || '',
  };
  for (const k in map) t = t.split(k).join(map[k]);
  const parts = parseCmd(t);
  return { exe: parts[0] || 'unity', args: parts.slice(1), display: t };
}

function composeCommand(engine, profile, ctx) {
  ctx.profile = profile;
  return engine.mode === 'template' ? composeTemplate(engine, ctx) : composeBatchmode(engine, ctx);
}

// ─────────────────────────── 构建任务（队列 + SSE） ───────────────────────────

let job = null;
let check = null; // 环境编译检测任务（独立于打包 job，二者互斥运行）
let vcs = null;   // 版本管理更新任务（与批量构建 / 环境检测互斥运行）
const sseClients = new Set();

function broadcast(evt) {
  if (evt.type === 'job-end' && job) job.end = { ok: evt.ok, reason: evt.reason, message: evt.message };
  if (evt.type === 'check-end' && check) check.end = { ok: evt.ok, reason: evt.reason, message: evt.message };
  if (evt.type === 'vcs-end' && vcs) vcs.end = { ok: evt.ok, reason: evt.reason, message: evt.message, summary: evt.summary };
  const data = 'data: ' + JSON.stringify(evt) + '\n\n';
  for (const res of sseClients) { try { res.write(data); } catch { /* 断开 */ } }
}

function pushLine(idx, text) {
  if (!job) return;
  job.lines.push({ i: idx, t: text });
  if (job.lines.length > 2000) job.lines.splice(0, job.lines.length - 2000);
  broadcast({ type: 'line', index: idx, text });
}

function pushCheckLine(text) {
  if (!check) return;
  check.lines.push(text);
  if (check.lines.length > 2000) check.lines.splice(0, check.lines.length - 2000);
  broadcast({ type: 'check-line', text });
}

function killProc(child) {
  if (child && child.pid) {
    try {
      if (process.platform === 'win32') {
        spawnSync('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore', windowsHide: true });
      } else {
        try { process.kill(-child.pid, 'SIGKILL'); } catch { child.kill('SIGKILL'); }
      }
    } catch (e) { console.error('[kill]', e.message); }
  }
}

function killChild() {
  if (job) killProc(job.child);
}

function runOne(engine, profile, ctx, idx) {
  return new Promise(resolve => {
    const cmd = composeCommand(engine, profile, ctx);
    try {
      if (fs.existsSync(ctx.dir)) {
        if (!ctx.isSharedDir) {
          for (const f of fs.readdirSync(ctx.dir)) {
            if (/\.(apk|aab|exe|zip)$/i.test(f)) { try { fs.unlinkSync(path.join(ctx.dir, f)); } catch { /* */ } }
          }
        } else if (ctx.output && fs.existsSync(ctx.output)) {
          try {
            const st = fs.statSync(ctx.output);
            if (st.isDirectory()) { /* 保留子目录 */ }
            else fs.unlinkSync(ctx.output);
          } catch { /* */ }
        }
      }
    } catch { /* */ }
    fs.mkdirSync(ctx.dir, { recursive: true });
    broadcast({ type: 'profile-start', index: idx, name: profile.name, target: profile.targetName, cmd: cmd.display, output: ctx.output, versionCode: ctx.versionCode || ctx.buildNo || null, dev: !!ctx.dev, buildAddressables: !!ctx.buildAddressables, androidBuildKind: ctx.androidBuildKind || 'apk' });

    const pstate = { stage: '启动…', pct: null, lastEmit: 0, lastStage: '', lastPct: -1 };
    if (job && job.progress) job.progress[idx] = { stage: pstate.stage, pct: null };

    let logFd = null;
    try { logFd = fs.openSync(ctx.logFile, 'w'); } catch { /* */ }

    let child;
    try {
      child = spawn(cmd.exe, cmd.args, {
        cwd: ctx.projectAbs,
        windowsHide: true,
        stdio: ['ignore', logFd === null ? 'ignore' : logFd, logFd === null ? 'ignore' : logFd],
      });
    } catch (e) {
      pushLine(idx, '\n[错误] 无法启动进程: ' + e.message + '\n');
      if (logFd !== null) { try { fs.closeSync(logFd); } catch { /* */ } }
      resolve({ ok: false, exitCode: -1 });
      return;
    }
    job.child = child;
    job.exitCode = null;

    let lastSize = 0;
    const tail = setInterval(() => {
      let st;
      try { st = fs.statSync(ctx.logFile); } catch { return; }
      if (!st || st.size <= lastSize) return;
      try {
        const fd = fs.openSync(ctx.logFile, 'r');
        const buf = Buffer.alloc(st.size - lastSize);
        fs.readSync(fd, buf, 0, buf.length, lastSize);
        fs.closeSync(fd);
        lastSize = st.size;
        const text = buf.toString('utf8');
        pushLine(idx, text);

        const pb = text.match(/DisplayProgressbar:\s*([^\r\n]+)/g);
        if (pb && pb.length) pstate.stage = pb[pb.length - 1].replace(/^DisplayProgressbar:\s*/, '').trim();
        const tk = text.match(/\[(\d+)\/(\d+)\s+\d+s\]/g);
        if (tk && tk.length) {
          const mm = tk[tk.length - 1].match(/\[(\d+)\/(\d+)/);
          if (mm && +mm[2]) pstate.pct = Math.round(+mm[1] / +mm[2] * 100);
        }
        const now = Date.now();
        const changed = pstate.stage !== (pstate.lastStage || '') || pstate.pct !== pstate.lastPct;
        if (changed && now - pstate.lastEmit >= 300) {
          pstate.lastEmit = now;
          pstate.lastStage = pstate.stage;
          pstate.lastPct = pstate.pct;
          if (job && job.progress) job.progress[idx] = { stage: pstate.stage, pct: pstate.pct };
          broadcast({ type: 'progress', index: idx, stage: pstate.stage, pct: pstate.pct });
        }
      } catch { /* */ }
    }, 200);

    child.on('close', code => {
      clearInterval(tail);
      if (logFd !== null) { try { fs.closeSync(logFd); } catch { /* */ } }
      try {
        let st = fs.statSync(ctx.logFile);
        if (st && st.size > lastSize) {
          const fd = fs.openSync(ctx.logFile, 'r');
          const buf = Buffer.alloc(st.size - lastSize);
          fs.readSync(fd, buf, 0, buf.length, lastSize);
          fs.closeSync(fd);
          pushLine(idx, buf.toString('utf8'));
        }
      } catch { /* */ }

      job.exitCode = code;
      job.child = null;
      resolve({ ok: code === 0, exitCode: code });
    });
  });
}

function writeGradleProxyTemplate(projectAbs, proxy) {
  const up = fwd(projectAbs);
  const lines = [
    'org.gradle.jvmargs=-Xmx4096m -XX:MaxMetaspaceSize=1024m',
    'org.gradle.parallel=true',
    'unityStreamingAssets=**STREAMING_ASSETS**',
    'unityProjectPath=' + up,
    'unity.projectPath=' + up,
    'android.useAndroidX=true',
    'android.enableJetifier=true',
    '',
  ];
  const en = proxy && proxy.enabled && proxy.host;
  if (en) {
    const host = proxy.host || '127.0.0.1';
    const port = proxy.port || '10808';
    lines.push('# Local proxy config');
    lines.push('systemProp.http.proxyHost=' + host);
    lines.push('systemProp.http.proxyPort=' + port);
    lines.push('systemProp.https.proxyHost=' + host);
    lines.push('systemProp.https.proxyPort=' + port);
  }
  const file = path.join(projectAbs, 'Assets', 'Plugins', 'Android', 'gradleTemplate.properties');
  fs.writeFileSync(file, lines.join('\n') + '\n', 'utf8');
  return { file, enabled: !!en };
}

function extractErrorExcerpt(logText) {
  const lines = logText.split(/\r?\n/);
  const strong = /FAILURE:|What went wrong|Execution failed|BUILD FAILED|GradleInvokationException|UnityException|error CS\d+|Exception:/;
  let lastStrong = -1;
  lines.forEach((l, i) => { if (strong.test(l)) lastStrong = i; });
  if (lastStrong >= 0) {
    const start = Math.max(0, lastStrong - 3);
    return lines.slice(start, Math.min(lines.length, start + 80)).join('\n').slice(-6000);
  }
  return lines.slice(-150).join('\n').slice(-6000);
}

async function aiAnalyze(body) {
  const ai = config.ai || {};
  if (!ai.enabled || !ai.apiKey) return { ok: false, error: '未启用 AI 或未填写 API Key（③ 引擎设置 → AI 分析）' };

  let target = null;
  if (body && body.name) target = job && job.queue.find(q => q.name === body.name);
  else if (job && job.lastFail) target = job.lastFail;
  if (!target) return { ok: false, error: '没有可分析的失败记录（请先运行一次失败的构建）' };

  let logText = '';
  try { logText = fs.readFileSync(target.logFile, 'utf8'); } catch { /* */ }
  if (!logText && target.output) {
    try { logText = fs.readFileSync(path.join(target.output, 'build.log'), 'utf8'); } catch { /* */ }
  }
  if (!logText) return { ok: false, error: '找不到构建日志: ' + (target.logFile || target.output || '未知') };

  const excerpt = extractErrorExcerpt(logText);
  const baseUrl = (ai.baseUrl || 'https://api.deepseek.com').replace(/\/+$/, '');
  const url = baseUrl + '/chat/completions';
  const sys = '你是一名资深的 Unity 构建排障专家，擅长 Android 动态壁纸 APK、iOS Xcode、Gradle、IL2CPP、keystore 签名、资源裁剪与网络依赖问题。用户提供构建失败日志摘录，请用中文回答：1) 失败原因（一句话概括）；2) 具体修复建议（分步骤、可直接执行）；3) 若是网络/依赖/签名问题请明确指出。保持简洁专业，不要客套。';
  const user = `构建目标: ${target.targetName || 'Android'}\nProfile: ${target.name}\n输出: ${target.output || ''}\n\n--- 构建日志摘录 ---\n${excerpt}\n--- 结束 ---`;

  try {
    const ctrl = new AbortController();
    const to = setTimeout(() => ctrl.abort(), 90000);
    const resp = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'Authorization': 'Bearer ' + ai.apiKey },
      body: JSON.stringify({
        model: ai.model || 'deepseek-chat',
        messages: [{ role: 'system', content: sys }, { role: 'user', content: user }],
        temperature: 0.3,
        max_tokens: 2000,
      }),
      signal: ctrl.signal,
    });
    clearTimeout(to);
    if (!resp.ok) {
      const t = await resp.text().catch(() => '');
      return { ok: false, error: 'AI 接口返回 ' + resp.status + ': ' + t.slice(0, 300) };
    }
    const data = await resp.json();
    const text = (data.choices && data.choices[0] && data.choices[0].message && data.choices[0].message.content) || '';
    if (!text) return { ok: false, error: 'AI 返回内容为空' };
    return { ok: true, analysis: text.trim() };
  } catch (e) {
    return { ok: false, error: '调用 AI 失败: ' + e.message };
  }
}

async function startJob(req) {
  if (job && (job.state === 'running' || job.state === 'starting')) {
    if (!job.child || (job.child.killed || job.child.exitCode !== null)) {
      console.warn('[job] 检测到残留状态，自动重置任务状态');
      job.state = 'done';
    } else {
      return { ok: false, error: '已有打包任务正在运行中，请先等待或点击「终止构建」' };
    }
  }
  if (check && (check.state === 'running' || check.state === 'starting')) {
    if (!check.child || (check.child.killed || check.child.exitCode !== null)) {
      check.state = 'done';
    } else {
      return { ok: false, error: '环境编译检测进行中，请先停止检测' };
    }
  }
  if (vcs && (vcs.state === 'running' || vcs.state === 'starting')) {
    if (!vcs.child || (vcs.child.killed || vcs.child.exitCode !== null)) {
      vcs.state = 'done';
    } else {
      return { ok: false, error: '版本管理更新进行中，请先停止更新' };
    }
  }

  const projectAbs = resolveProject(req.projectPath);
  if (!isValidProject(projectAbs)) return { ok: false, error: '项目路径无效: ' + req.projectPath };

  const profiles = (req.profiles || []).filter(p => p && p.assetPath);
  if (!profiles.length) return { ok: false, error: '未选择任何 Profile' };

  const engine = req.engine || {};
  if (engine.mode === 'template') {
    if (!(engine.template || '').trim()) return { ok: false, error: '模板引擎下未填写命令模板' };
  } else if (!engine.unityExe || !fs.existsSync(engine.unityExe)) {
    return { ok: false, error: '未配置有效的 Unity.exe 路径（或该路径不存在）' };
  }

  const outBase = req.outputBase ? (path.isAbsolute(req.outputBase) ? req.outputBase : path.join(projectAbs, req.outputBase)) : path.join(projectAbs, 'Builds');
  const successDirInput = (req.successDir || '').trim();
  if (successDirInput) config.successDir = successDirInput;
  const stopOnError = req.stopOnError !== false;
  const buildNumberRaw = Number(req.buildNumber);
  const buildNumberAuto = !Number.isFinite(buildNumberRaw) || buildNumberRaw === -1;

  job = { state: 'starting', queue: profiles, index: 0, lines: [], progress: {}, outputs: [], child: null, exitCode: null, timer: null, stopOnError, cancel: false, lastFail: null };
  broadcast({ type: 'job-start', count: profiles.length });

  if (profiles.some(p => p.target === 13)) {
    try {
      const t = writeGradleProxyTemplate(projectAbs, req.proxy || {});
      broadcast({ type: 'notice', text: t.enabled
        ? `[代理] 已写入 Gradle 代理模板: ${req.proxy.host}:${req.proxy.port}`
        : '[代理] 未启用，Gradle 将直连（国内可能下载 AGP 失败）' });
    } catch (e) {
      broadcast({ type: 'notice', text: '[代理] 写入 gradleTemplate.properties 失败: ' + e.message });
    }
  }

  const run = async () => {
    job.state = 'running';
    try {
      for (let i = 0; i < profiles.length; i++) {
        if (job.cancel) { broadcast({ type: 'job-end', ok: false, reason: 'cancelled' }); return; }
        job.index = i;
        const profile = profiles[i];

        // 每个构建配置单独配置「版本更新组」：构建该配置前先更新（revert → 分支切换 → pull/update）
        const preGids = Array.isArray(profile.vcsGroupIds) ? profile.vcsGroupIds.map(String) : [];
        const preNodes = preGids.length ? nodeIdsOfGroups(preGids) : [];
        if (preNodes.length) {
          const pre = await runPreBuildVcsUpdate(preNodes, groupNamesOf(preGids), `构建配置「${profile.name}」`, i);
          if (job.cancel) { broadcast({ type: 'job-end', ok: false, reason: 'cancelled' }); return; }
          if (!pre.continue) {
            if (pre.reason === 'error') { broadcast({ type: 'job-end', ok: false, reason: 'error', message: pre.message }); return; }
            if (job.stopOnError) {
              broadcast({ type: 'job-end', ok: false, reason: 'vcsPreFail', index: i });
              return;
            }
            broadcast({ type: 'notice', text: `[版本管理] 「${profile.name}」构建前版本更新存在失败项，已跳过该构建配置（失败即停已关闭）` });
            broadcast({ type: 'profile-end', index: i, ok: false, skipped: true });
            continue;
          }
        }

        const sign = profile.sign || {};
        const customDir = profile.customBuildDir || sign.customBuildDir || null;
        const resolvedAndroidKind = String(profile.androidBuildKind || sign.androidBuildKind || req.androidBuildKind || 'apk').toLowerCase();
        const ctx = makeOutputs(profile, outBase, customDir, resolvedAndroidKind);
        ctx.projectAbs = projectAbs;
        ctx.outBase = outBase;
        ctx.successDir = config.successDir || '构建成功';
        ctx.sign = sign;
        ctx.dev = profile.dev != null ? profile.dev : (sign.dev != null ? sign.dev : !!req.dev);
        ctx.buildAddressables = profile.buildAddressables != null ? profile.buildAddressables : (sign.buildAddressables != null ? sign.buildAddressables : !!req.buildAddressables);
        ctx.addressablesMethod = profile.addressablesMethod || sign.addressablesMethod || req.addressablesMethod || null;
        ctx.androidBuildKind = resolvedAndroidKind;
        ctx.nameTemplate = profile.nameTemplate || sign.nameTemplate || req.artifactNameTemplate || cfg('artifactNameTemplate', '');
        ctx.autoZip = profile.autoZip != null ? profile.autoZip : (sign.autoZip != null ? sign.autoZip : (req.autoZip != null ? req.autoZip : cfg('defaultAutoZip', true)));

        const buildNo = buildNumberAuto ? ((Number(config.lastVersionCode) || 0) + 1) : buildNumberRaw;
        ctx.buildNo = buildNo;
        if (profile.target === 13) ctx.versionCode = buildNo;
        const t0 = Date.now();
        const r = await runOne(engine, profile, ctx, i);
        const duration = Date.now() - t0;
        broadcast({ type: 'profile-end', index: i, ok: r.ok, exitCode: r.exitCode, versionCode: ctx.versionCode || null, buildNo: ctx.buildNo || null });
        if (r.ok && (ctx.versionCode || ctx.buildNo)) {
          config.lastVersionCode = ctx.versionCode || ctx.buildNo;
          saveConfig();
        }
        if (r.ok) {
          let out = null;
          try {
            out = finalizeArtifact(profile, ctx, duration);
          } catch (e) {
            broadcast({ type: 'notice', text: '\n[警告] 产物归档失败: ' + e.message + '\n' });
          }
          if (!out) out = { name: profile.name, targetName: profile.targetName, file: ctx.output, dir: ctx.dir, time: Date.now() };
          job.outputs = job.outputs || [];
          job.outputs.unshift(out);
          broadcast({ type: 'output', name: out.name, targetName: out.targetName, file: out.file, dir: out.dir,
            version: out.version || null, versionCode: out.versionCode != null ? out.versionCode : null,
            date: out.date || null, kind: out.kind || null, size: out.size || null, dev: out.dev });
        }
        if (!r.ok) {
          job.lastFail = { name: profile.name, targetName: profile.targetName, logFile: ctx.logFile, output: ctx.output, exitCode: r.exitCode };
          if (job.cancel) { broadcast({ type: 'job-end', ok: false, reason: 'cancelled' }); return; }
          if (job.stopOnError) {
            broadcast({ type: 'job-end', ok: false, reason: 'stopOnError', index: i });
            return;
          }
        }
      }
      broadcast({ type: 'job-end', ok: true });
    } catch (e) {
      console.error('[job] 异常:', e);
      try { broadcast({ type: 'job-end', ok: false, reason: 'error', message: e.message }); } catch { /* */ }
    } finally {
      job.state = 'done';
      broadcast({ type: 'done', end: job.end });
    }
  };

  run();
  return { ok: true };
}

function stopJob() {
  if (!job || job.state === 'done') return { ok: false, error: '没有运行中的任务' };
  job.cancel = true;
  killChild();
  if (vcs && vcs.preBuild) killProc(vcs.child); // 构建前版本更新阶段：一并强杀正在执行的 svn/git 进程
  return { ok: true };
}

// ─────────────────────────── 环境编译检测（环境线） ───────────────────────────

/**
 * 环境编译检测：对用户指定的多条环境线（BuildTarget：Win x64/x86 / Android / iOS / macOS）
 * 逐条调用 Unity.exe -batchmode 执行一次真实编译检测
 * （Unity 端入口 TBuildTool.Editor.CompileCheck.Run：校验支持性 → 切换目标 → 触发脚本编译 → 检查错误）。
 * 每条环境线一个 Unity 进程，产出独立的 result JSON 与日志；结果经 SSE 实时推送。
 */

function composeCheckCommand(engine, projectAbs, target, files, timeoutMinutes) {
  const args = ['-batchmode'];
  if (engine.nographics !== false) args.push('-nographics');
  // 注意：编译检测需等待编译完成，不能传 -quit，由 Unity 端 EditorApplication.Exit 收尾
  args.push('-projectPath', projectAbs,
    '-executeMethod', 'TBuildTool.Editor.CompileCheck.Run',
    '-target', String(target),
    '-resultFile', files.json,
    '-timeout', String(timeoutMinutes || 20),
    '-logFile', files.log);
  return { exe: engine.unityExe, args, display: [quote(engine.unityExe)].concat(args.map(quote)).join(' ') };
}

function readCheckResult(file) {
  try {
    const txt = fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, '');
    const o = JSON.parse(txt);
    return {
      target: Number(o.target) || null,
      name: o.name || '',
      group: o.group || '',
      ok: !!o.ok,
      status: o.status || 'error',
      ms: Number(o.ms) || null,
      message: o.message || '',
      unityVersion: o.unityVersion || '',
      finishedAt: o.finishedAt || '',
    };
  } catch { return null; }
}

function checkOverall(results) {
  const list = Object.values(results || {});
  if (!list.length) return false;
  return !list.some(r => ['fail', 'error', 'timeout'].includes(r.status));
}

function summarizeCheck(results) {
  const s = { total: 0, ok: 0, fail: 0, unsupported: 0, timeout: 0, error: 0, cancelled: 0, running: 0 };
  for (const r of Object.values(results || {})) {
    s.total++;
    if (s[r.status] != null) s[r.status]++;
  }
  return s;
}

/**
 * 单条环境线：启动 Unity 进程，尾随日志实时推送，进程退出后读取结果 JSON。
 */
function runOneCheck(engine, projectAbs, target, st, idx) {
  return new Promise(resolve => {
    const files = st.files[idx];
    const targetName = TARGETS[target] || ('BuildTarget ' + target);
    const cmd = composeCheckCommand(engine, projectAbs, target, files, st.timeoutMinutes);
    st.results[target] = { target, name: targetName, status: 'running', ms: null, message: '检测中…（启动 Unity 编译）', ok: false };
    broadcast({ type: 'check-target-start', index: idx, target, name: targetName, logFile: files.log, resultFile: files.json, cmd: cmd.display });
    pushCheckLine(`\n════════ 环境线 ${idx + 1}/${st.targets.length}：${targetName} (target ${target}) ════════\n命令: ${cmd.display}\n结果文件: ${files.json}\n`);

    let logFd = null;
    try { logFd = fs.openSync(files.log, 'w'); } catch { /* */ }

    let child;
    try {
      child = spawn(cmd.exe, cmd.args, {
        cwd: ROOT,
        windowsHide: true,
        stdio: ['ignore', logFd === null ? 'ignore' : logFd, logFd === null ? 'ignore' : logFd],
      });
    } catch (e) {
      pushCheckLine('\n[错误] 无法启动 Unity 进程: ' + e.message + '\n');
      if (logFd !== null) { try { fs.closeSync(logFd); } catch { /* */ } }
      st.results[target] = { target, name: targetName, status: 'error', ms: null, message: '无法启动 Unity 进程: ' + e.message, ok: false };
      broadcast({ type: 'check-result', target, result: st.results[target] });
      resolve({ ok: false });
      return;
    }
    st.child = child;

    let lastSize = 0;
    const tail = setInterval(() => {
      let s;
      try { s = fs.statSync(files.log); } catch { return; }
      if (!s || s.size <= lastSize) return;
      try {
        const fd = fs.openSync(files.log, 'r');
        const buf = Buffer.alloc(s.size - lastSize);
        fs.readSync(fd, buf, 0, buf.length, lastSize);
        fs.closeSync(fd);
        lastSize = s.size;
        pushCheckLine(buf.toString('utf8'));
      } catch { /* */ }
    }, 200);

    child.on('close', code => {
      clearInterval(tail);
      if (logFd !== null) { try { fs.closeSync(logFd); } catch { /* */ } }
      try {
        const s2 = fs.statSync(files.log);
        if (s2 && s2.size > lastSize) {
          const fd = fs.openSync(files.log, 'r');
          const buf = Buffer.alloc(s2.size - lastSize);
          fs.readSync(fd, buf, 0, buf.length, lastSize);
          fs.closeSync(fd);
          pushCheckLine(buf.toString('utf8'));
        }
      } catch { /* */ }

      let res = readCheckResult(files.json);
      if (!res || !res.target) {
        res = {
          target, name: targetName,
          status: code === 0 ? 'ok' : 'error',
          ok: code === 0,
          ms: null,
          message: code === 0
            ? 'Unity 已退出（未写入结果文件）'
            : 'Unity 进程异常退出（exit=' + code + '）。可能项目脚本编译失败或方法入口缺失，详见日志。',
        };
      }
      st.results[target] = res;
      st.child = null;
      broadcast({ type: 'check-result', target, result: res });
      pushCheckLine(`\n[环境线 ${targetName}] 状态: ${res.status}${res.ms != null ? ' · 耗时 ' + (res.ms / 1000).toFixed(1) + 's' : ''}${res.message ? ' · ' + res.message : ''}\n`);
      resolve({ ok: res.status === 'ok' || res.status === 'unsupported' });
    });
  });
}

async function startCheck(req) {
  if (check && (check.state === 'running' || check.state === 'starting')) return { ok: false, error: '已有环境检测在运行' };
  if (job && (job.state === 'running' || job.state === 'starting')) return { ok: false, error: '批量构建进行中，请先停止构建' };
  if (vcs && (vcs.state === 'running' || vcs.state === 'starting')) return { ok: false, error: '版本管理更新进行中，请先停止更新' };

  const projectAbs = resolveProject(req.projectPath);
  if (!isValidProject(projectAbs)) return { ok: false, error: '项目路径无效: ' + req.projectPath };

  const engine = req.engine || {};
  if (!engine.unityExe || !fs.existsSync(engine.unityExe)) return { ok: false, error: '未配置有效的 Unity.exe 路径（可在「引擎与高级配置」中设置或自动探测）' };

  const raw = (req.targets || []).filter(t => ALLOWED_TARGETS.has(Number(t))).map(Number);
  if (!raw.length) return { ok: false, error: '未选择任何环境线（至少勾选一条目标平台）' };
  const targets = [...new Set(raw)];
  const timeoutMinutes = Math.max(1, Number(req.timeoutMinutes) || 20);

  const runDir = path.join(CHECK_RESULTS_DIR, 'check-' + formatNameTs(new Date()));
  fs.mkdirSync(runDir, { recursive: true });
  const files = targets.map(t => ({ target: t, json: path.join(runDir, 'target-' + t + '.json'), log: path.join(runDir, 'target-' + t + '.log') }));

  check = { state: 'starting', targets, index: 0, results: {}, lines: [], child: null, cancel: false, end: null, timeoutMinutes, runDir, files, projectAbs, engine, startedAt: new Date().toISOString() };
  pushCheckLine(`\n════════ 环境编译检测启动：${targets.map(t => TARGETS[t] || t).join(' / ')}（共 ${targets.length} 条环境线）════════\n项目: ${projectAbs}\n结果目录: ${runDir}\n`);
  broadcast({ type: 'check-start', count: targets.length, targets, runDir });

  const run = async () => {
    check.state = 'running';
    try {
      for (let i = 0; i < targets.length; i++) {
        if (check.cancel) { broadcast({ type: 'check-end', ok: false, reason: 'cancelled' }); return; }
        check.index = i;
        const target = targets[i];

        // 单条环境线超时看门狗（Unity 侧也有超时，这里兜底强杀）
        const timer = setTimeout(() => {
          if (check && check.child && check.index === i) {
            killProc(check.child);
            check.results[target] = {
              target, name: TARGETS[target] || ('BuildTarget ' + target),
              status: 'timeout', ok: false, ms: null,
              message: '检测超时（超过 ' + timeoutMinutes + ' 分钟 / 环境线），已强制终止',
            };
            broadcast({ type: 'check-result', target, result: check.results[target] });
          }
        }, timeoutMinutes * 60 * 1000 + 120000);

        await runOneCheck(engine, projectAbs, target, check, i);
        clearTimeout(timer);
        if (check.cancel) { broadcast({ type: 'check-end', ok: false, reason: 'cancelled' }); return; }
      }

      try {
        const summary = {
          ok: checkOverall(check.results),
          summary: summarizeCheck(check.results),
          startedAt: check.startedAt,
          finishedAt: new Date().toISOString(),
          targetOrder: targets,
          results: targets.map(t => check.results[t]).filter(Boolean),
        };
        fs.writeFileSync(path.join(check.runDir, 'summary.json'), JSON.stringify(summary, null, 2), 'utf8');
      } catch (e) { console.error('[check] 写 summary 失败:', e.message); }

      const overall = checkOverall(check.results);
      const s = summarizeCheck(check.results);
      pushCheckLine(`\n════════ 环境编译检测结束：通过 ${s.ok} ｜ 失败 ${s.fail}${s.error ? ' ｜ 异常 ' + s.error : ''}${s.timeout ? ' ｜ 超时 ' + s.timeout : ''}${s.unsupported ? ' ｜ 不支持 ' + s.unsupported : ''} ════════\n`);
      broadcast({ type: 'check-end', ok: overall, summary: s });
    } catch (e) {
      console.error('[check] 异常:', e);
      try { broadcast({ type: 'check-end', ok: false, reason: 'error', message: e.message }); } catch { /* */ }
    } finally {
      check.state = 'done';
    }
  };

  run();
  return { ok: true, runDir };
}

function stopCheck() {
  if (!check || check.state === 'done') return { ok: false, error: '没有运行中的环境检测' };
  check.cancel = true;
  killProc(check.child);
  return { ok: true };
}

// ─────────────────────────── 版本管理（SVN / Git 节点与分组） ───────────────────────────

/**
 * 版本管理：每个「节点」对应一条本地路径 + 版本控制类型（自动检测 Git / SVN，或手动指定），
 * 「分组」把若干节点聚合为组。支持单节点 / 分组 / 全部一键更新；
 * 任何更新前都会先「还原未提交更改」——Git：`git reset --hard`；SVN：`svn revert -R .`——
 * 再执行 `git pull` / `svn update`。路径为空或未检测到工作副本的节点一律跳过，不执行任何更新。
 * 节点可配置「目标分支」：配置分支与当前分支不一致时，更新顺序为
 * 先还原未提交更改 → 尝试切换分支（Git：checkout，失败则基于远端创建本地跟踪分支；
 * SVN：svn switch 到分支 URL）→ 再 pull / update。
 * 每个「构建配置」（队列中的 Profile）可单独配置构建前要更新的版本更新组。
 * 节点与分组配置存于 config.json 的 `vcs` 字段。
 */

function vcsConfig() {
  const c = config.vcs;
  return c && typeof c === 'object'
    ? { nodes: Array.isArray(c.nodes) ? c.nodes : [], groups: Array.isArray(c.groups) ? c.groups : [] }
    : { nodes: [], groups: [] };
}

function saveVcsConfig(cfg) {
  config.vcs = {
    nodes: Array.isArray(cfg.nodes) ? cfg.nodes : [],
    groups: Array.isArray(cfg.groups) ? cfg.groups : [],
  };
  saveConfig();
}

function resolveVcsPath(p) {
  // 与 resolveProject 一致：相对路径基于本工具目录（TBuildTool\web）
  if (!p || !String(p).trim()) return null;
  return path.isAbsolute(p) ? path.normalize(p) : path.resolve(ROOT, p);
}

/**
 * 自动检测指定路径的版本控制类型：依次向上（最多 12 层）查找 .git / .svn 标记。
 * 显式指定类型的节点不再探测（见 nodeType）。
 */
function detectVcsType(absPath) {
  if (!absPath || !fs.existsSync(absPath)) return null;
  let cur = absPath;
  for (let i = 0; i < 12; i++) {
    try {
      if (fs.existsSync(path.join(cur, '.git'))) return 'git';
      if (fs.existsSync(path.join(cur, '.svn'))) return 'svn';
    } catch { break; }
    const parent = path.dirname(cur);
    if (parent === cur) break;
    cur = parent;
  }
  return null;
}

/** 节点显式指定的类型（git / svn）；auto 返回 null 表示需要自动检测 */
function nodeType(node) {
  const t = String((node && node.type) || 'auto').toLowerCase();
  return (t === 'git' || t === 'svn') ? t : null;
}

function runSync(cmd, args, cwd, timeoutMs) {
  try {
    return spawnSync(cmd, args, {
      cwd, encoding: 'utf8', timeout: timeoutMs || 10000, windowsHide: true,
      env: { ...process.env, GIT_TERMINAL_PROMPT: '0' },
    });
  } catch (e) {
    return { error: e.message };
  }
}

function gitInfo(abs) {
  const out = {
    type: 'git', branch: '', revision: '', dirty: false, dirtyCount: 0, error: null,
    hasRemote: false, remoteName: '', remoteOnline: false,
  };
  const br = runSync('git', ['rev-parse', '--abbrev-ref', 'HEAD'], abs);
  if (br.error) { out.error = br.error; return out; }
  out.branch = String(br.stdout || '').trim() || 'HEAD';
  const rev = runSync('git', ['rev-parse', '--short=8', 'HEAD'], abs);
  out.revision = String(rev.stdout || '').trim();
  const st = runSync('git', ['status', '--porcelain'], abs, 20000);
  if (!st.error && st.status === 0) {
    const lines = String(st.stdout || '').split(/\r?\n/).filter(Boolean);
    out.dirty = lines.length > 0;
    out.dirtyCount = lines.length;
  }
  // 远端检测：是否有远端服务器 + 服务器是否在线
  const rem = gitRemoteStatus(abs);
  out.hasRemote = rem.hasRemote;
  out.remoteName = rem.remoteName || '';
  out.remoteOnline = rem.online;
  return out;
}

/** Git 远端连通性检测：无远端 → { hasRemote:false }；有远端则 ls-remote 验证在线 */
function gitRemoteStatus(abs) {
  const rem = runSync('git', ['remote'], abs, 8000);
  if (rem.error) return { hasRemote: false, remoteName: '', online: false, reason: '读取 remote 失败: ' + rem.error };
  const names = String(rem.stdout || '').split(/\r?\n/).filter(Boolean);
  if (!names.length) return { hasRemote: false, remoteName: '', online: false, reason: '无远端服务器（只有本地仓库）' };
  const remote = names[0];
  const ls = runSync('git', ['ls-remote', '--heads', remote], abs, 12000);
  const online = !ls.error && ls.status === 0;
  return {
    hasRemote: true, remoteName: remote, online,
    reason: online ? '' : ('远端不可达: ' + String(ls.stderr || ls.error || '').trim().slice(0, 200)),
  };
}

function svnInfo(abs) {
  const out = {
    type: 'svn', branch: '', root: '', revision: '', dirty: false, dirtyCount: 0, error: null,
    hasRemote: true, remoteOnline: false,
  };
  const info = runSync('svn', ['info', '--xml'], abs, 20000);
  if (info.error) { out.error = info.error; return out; }
  const text = String(info.stdout || '');
  const rev = text.match(/revision="(\d+)"/);
  const url = text.match(/<url>([^<]+)<\/url>/);
  const root = text.match(/<root>([^<]+)<\/root>/);
  out.revision = rev ? rev[1] : '';
  out.branch = url ? decodeURIComponent(url[1]) : '';
  out.root = root ? decodeURIComponent(root[1]) : '';
  // 服务器连通性：对仓库 URL 做一次显式 info（工作副本本地 info 可能离线可用）
  if (url) {
    const on = runSync('svn', ['--non-interactive', 'info', url[1]], abs, 15000);
    out.remoteOnline = !on.error && on.status === 0;
  } else {
    out.remoteOnline = false;
  }
  const st = runSync('svn', ['status'], abs, 20000);
  if (!st.error && st.status === 0) {
    const lines = String(st.stdout || '').split(/\r?\n/).filter(l => l.trim());
    out.dirty = lines.length > 0;
    out.dirtyCount = lines.length;
  }
  return out;
}

/**
 * 解析 SVN 分支：
 * 带协议前缀（http:// https:// svn:// file:// ...）= 直接作为完整 URL；
 * 否则视为仓库内相对路径，拼到仓库根（root）下，如 branches/dev → <root>/branches/dev。
 */
function resolveSvnBranchUrl(branch, root, currentUrl) {
  const b = String(branch || '').trim();
  if (!b) return null;
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(b)) return b;
  const base = root || (currentUrl ? currentUrl.replace(/\/+(trunk|branches|tags)\/.*$/, '') : null);
  if (!base) return null;
  return base.replace(/\/+$/, '') + '/' + b.replace(/^\/+/, '');
}

function normalizeUrl(u) {
  return String(u || '').replace(/\/+$/, '').replace(/\\/g, '/').toLowerCase();
}

/** Git 可用分支列表：本地分支优先，再补远端跟踪分支（去掉 remote 前缀、去重），供下拉选择 */
function gitBranches(abs) {
  const out = [];
  const locals = [], remotes = new Set();
  const r1 = runSync('git', ['for-each-ref', '--format=%(refname:short)', 'refs/heads'], abs, 10000);
  if (!r1.error && r1.status === 0) {
    for (const line of String(r1.stdout || '').split(/\r?\n/).map(s => s.trim()).filter(Boolean)) {
      if (line !== 'HEAD') locals.push(line);
    }
  }
  const r2 = runSync('git', ['for-each-ref', '--format=%(refname:short)', 'refs/remotes'], abs, 10000);
  if (!r2.error && r2.status === 0) {
    for (const line of String(r2.stdout || '').split(/\r?\n/).map(s => s.trim()).filter(Boolean)) {
      const rest = line.split('/').slice(1).join('/'); // refs/remotes/origin/main -> main
      if (rest && rest !== 'HEAD') remotes.add(rest);
    }
  }
  const seen = new Set();
  for (const b of locals.concat([...remotes])) {
    if (!seen.has(b)) { seen.add(b); out.push(b); }
  }
  return out;
}

/** SVN 分支列表：在线时枚举 <仓库根>/branches，另附 trunk；当前 URL 本身是分支也补上 */
function svnBranches(abs, info) {
  const out = [];
  if (info && info.remoteOnline && info.root) {
    const ls = runSync('svn', ['--non-interactive', 'ls', info.root.replace(/\/+$/, '') + '/branches'], abs, 12000);
    if (!ls.error && ls.status === 0) {
      for (const line of String(ls.stdout || '').split(/\r?\n/).map(s => s.trim().replace(/\/+$/, '')).filter(Boolean)) {
        if (line !== '.' && line !== '..') out.push(line);
      }
    }
    if (!out.includes('trunk')) out.unshift('trunk');
  }
  if (info && info.branch) {
    const m = String(info.branch).match(/\/(?:branches|tags)\/([^/]+)\/?$/);
    if (m && !out.includes(m[1])) out.push(m[1]);
    if (/\/trunk\/?$/.test(String(info.branch)) && !out.includes('trunk')) out.push('trunk');
  }
  return out;
}

function probeNode(node) {
  const abs = resolveVcsPath(node && node.path);
  const base = {
    id: node.id, name: node.name || node.id, path: (node && node.path) || '',
    nodeBranch: String((node && node.branch) || '').trim(),
    nodeRevert: String((node && node.revert) || '').trim(),
  };
  if (!abs || !fs.existsSync(abs)) {
    return Object.assign(base, { ok: true, type: 'none', root: '', branch: '', revision: '', dirty: false, dirtyCount: 0, hasRemote: false, remoteOnline: false, branchMatched: null, branchExists: null, branches: [], error: '路径为空或不存在' });
  }
  const type = nodeType(node) || detectVcsType(abs);
  if (!type) {
    return Object.assign(base, { ok: false, type: 'none', root: '', branch: '', revision: '', dirty: false, dirtyCount: 0, hasRemote: false, remoteOnline: false, branchMatched: null, branchExists: null, branches: [], error: '未检测到 Git / SVN 工作副本（路径下无 .git 或 .svn）' });
  }
  let info;
  try {
    info = type === 'git' ? gitInfo(abs) : svnInfo(abs);
  } catch (e) {
    return Object.assign(base, { ok: false, type, root: '', branch: '', revision: '', dirty: false, dirtyCount: 0, hasRemote: false, remoteOnline: false, branchMatched: null, branchExists: null, branches: [], error: '读取状态失败: ' + e.message });
  }

  // 分支情况检查：当前分支 / 配置分支是否一致；远端是否在线；远端是否存在该分支
  let branchMatched = null, branchExists = null;
  if (base.nodeBranch) {
    if (type === 'git') {
      const b = base.nodeBranch;
      branchMatched = String(info.branch || '').trim() === b;
      if (info.remoteOnline && info.remoteName) {
        const lsB = runSync('git', ['ls-remote', '--heads', info.remoteName, b], abs, 10000);
        branchExists = !lsB.error && lsB.status === 0 && /refs\/heads\//.test(String(lsB.stdout || ''));
      }
    } else {
      const targetUrl = resolveSvnBranchUrl(base.nodeBranch, info.root || null, info.branch || null);
      branchMatched = targetUrl ? (normalizeUrl(targetUrl) === normalizeUrl(info.branch)) : false;
      if (info.remoteOnline && targetUrl) {
        const lsB = runSync('svn', ['--non-interactive', 'ls', targetUrl], abs, 10000);
        branchExists = !lsB.error && lsB.status === 0;
      }
    }
  }

  return Object.assign(base, {
    ok: !info.error,
    type,
    root: info.root || '',
    branch: info.branch || '',
    revision: info.revision || '',
    dirty: !!info.dirty,
    dirtyCount: info.dirtyCount || 0,
    hasRemote: info.hasRemote !== false,
    remoteOnline: !!info.remoteOnline,
    branchMatched,
    branchExists,
    branches: type === 'git' ? gitBranches(abs) : svnBranches(abs, info),
    error: info.error ? ('读取状态失败: ' + info.error) : null,
  });
}

function pushVcsLine(text) {
  if (!vcs) return;
  vcs.lines.push(text);
  if (vcs.lines.length > 2000) vcs.lines.splice(0, vcs.lines.length - 2000);
  broadcast({ type: 'vcs-line', text });
}

function vcsCmdLine(cmd, args) { return quote(cmd) + ' ' + args.map(quote).join(' '); }

/**
 * 串行执行一条版本管理命令，stdout/stderr 实时经 SSE 推送；
 * 支持取消（killProc 强杀 + 看门狗超时兜底）。
 */
function runVcsProc(node, index, label, cmd, args, cwd, timeoutMs) {
  return new Promise(resolve => {
    pushVcsLine(`\n── ${label}：${vcsCmdLine(cmd, args)}\n`);
    let child;
    try {
      child = spawn(cmd, args, {
        cwd, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
        env: { ...process.env, GIT_TERMINAL_PROMPT: '0' },
      });
    } catch (e) {
      pushVcsLine('[错误] 无法启动进程: ' + e.message + '\n');
      resolve({ ok: false, exitCode: -1, error: e.message });
      return;
    }
    vcs.child = child;
    let buf = '';
    const onData = d => {
      const chunks = String(d);
      const all = buf + chunks;
      if (all.includes('\n')) {
        const lines = all.split(/\r?\n/);
        buf = lines.pop();
        for (const l of lines) if (l.trim()) pushVcsLine(l + '\n');
      } else {
        buf = all;
      }
    };
    if (child.stdout) child.stdout.on('data', onData);
    if (child.stderr) child.stderr.on('data', onData);
    child.on('error', e => pushVcsLine('[错误] ' + e.message + '\n'));

    let settled = false;
    const finish = (r) => { if (!settled) { settled = true; resolve(r); } };
    const timer = setTimeout(() => {
      killProc(child);
      finish({ ok: false, exitCode: -1, error: '超时（' + timeoutMs + 'ms）' });
    }, timeoutMs);
    child.on('close', code => {
      clearTimeout(timer);
      if (buf.trim()) pushVcsLine(buf + '\n');
      if (vcs && vcs.child === child) vcs.child = null;
      finish({ ok: code === 0, exitCode: code });
    });
  });
}

function vcsNodeResult(node, index, name, type, status, ok, message) {
  const r = { id: node.id, name, type, status, ok, message, finishedAt: new Date().toISOString() };
  if (vcs) vcs.results[node.id] = r;
  broadcast({ type: 'vcs-node-end', index, nodeId: node.id, name, type, status, ok, message });
  return r;
}

/**
 * 通过系统 shell 执行一条自定义命令（用于自定义还原指令）：
 * Windows: cmd /d /s /c "<command>"；其它平台: sh -c "<command>"。
 */
function runVcsShell(node, index, label, command, cwd, timeoutMs) {
  if (process.platform === 'win32') {
    return runVcsProc(node, index, label, 'cmd.exe', ['/d', '/s', '/c', command], cwd, timeoutMs);
  }
  return runVcsProc(node, index, label, '/bin/sh', ['-c', command], cwd, timeoutMs);
}

/** 节点自定义还原指令（配了就用它，否则返回 null 走默认普通还原） */
function nodeRevertCommand(node) {
  const c = String((node && node.revert) || '').trim();
  return c || null;
}

/**
 * 健壮的 Git 还原：先确认有提交（ unborn HEAD 时无可还原内容），
 * `git reset --hard` 失败（如文件被占用 / 只读）时回退尝试 `git checkout -- .`。
 */
async function gitRevert(node, index, abs, timeoutMs) {
  const head = runSync('git', ['rev-parse', '--verify', 'HEAD'], abs, 8000);
  if (head.error || head.status !== 0) {
    pushVcsLine('（仓库尚无提交记录，无未提交内容可还原）\n');
    return { ok: true, skipped: true };
  }
  const r1 = await runVcsProc(node, index, '还原未提交更改 (git reset --hard)', 'git', ['reset', '--hard'], abs, timeoutMs);
  if (r1.ok) return { ok: true, method: 'reset --hard' };
  const r2 = await runVcsProc(node, index, 'reset --hard 失败，尝试 git checkout -- .', 'git', ['checkout', '--', '.'], abs, timeoutMs);
  if (r2.ok) return { ok: true, method: 'checkout -- .' };
  pushVcsLine('（还原失败：可能有文件被占用（如已打开的编辑器 / 构建进程锁定）或处于只读状态，详见上方错误输出）\n');
  return { ok: false, error: r2.error || r1.error || ('退出码 ' + r2.exitCode) };
}

/**
 * 更新单个节点流程：
 *   1) 一律先「还原未提交更改」（Git 健壮还原 / SVN revert -R .）
 *   2) 检测远端：不在线或没有远端服务器 → 只执行 revert，不 pull / update
 *   3) 在线且有远端 → 配置分支与当前分支不一致时先还原再尝试切换分支
 *      （Git：checkout → 失败则 fetch 远端后基于远端创建本地跟踪分支；SVN：svn switch）
 *      再 pull / update
 * 路径为空 → skipped（不执行任何操作）；未检测到工作副本 → skipped。
 */
async function updateVcsNode(node, index, timeoutMs) {
  const name = node.name || node.id;
  const abs = resolveVcsPath(node.path);
  if (!abs || !fs.existsSync(abs)) {
    pushVcsLine('（路径为空或不存在，跳过节点，未执行任何更新操作）\n');
    return vcsNodeResult(node, index, name, 'none', 'skipped', false, '路径为空或不存在，跳过');
  }
  const type = nodeType(node) || detectVcsType(abs);
  if (!type) {
    pushVcsLine('（未检测到 Git / SVN 工作副本，跳过节点，未执行任何更新操作）\n');
    return vcsNodeResult(node, index, name, 'none', 'skipped', false, '未检测到 Git / SVN 工作副本');
  }

  if (type === 'git') {
    // 远端检测（在线才 pull；无远端 / 不在线 → 仅还原）
    const rem = gitRemoteStatus(abs);
    pushVcsLine(rem.hasRemote
      ? `（远端 ${rem.remoteName}：${rem.online ? '在线' : '不可达，仅执行还原，跳过 pull'}）\n`
      : '（无远端服务器，仅执行还原，跳过 pull）\n');

    // 还原未提交更改：优先使用节点自定义还原指令，否则默认普通还原
    const customRevert = nodeRevertCommand(node);
    if (customRevert) {
      const rr = await runVcsShell(node, index, '自定义还原指令', customRevert, abs, timeoutMs);
      if (vcs && vcs.cancel) return vcsNodeResult(node, index, name, type, 'cancelled', false, '已取消');
      if (!rr.ok) return vcsNodeResult(node, index, name, type, 'fail', false, '自定义还原指令执行失败（退出码 ' + rr.exitCode + '）');
    } else {
      const rv = await gitRevert(node, index, abs, timeoutMs);
      if (vcs && vcs.cancel) return vcsNodeResult(node, index, name, type, 'cancelled', false, '已取消');
      if (!rv.ok) return vcsNodeResult(node, index, name, type, 'fail', false, '还原未提交更改失败' + (rv.error ? '：' + rv.error : ''));
    }

    if (!rem.online) {
      return vcsNodeResult(node, index, name, type, 'partial', true,
        rem.hasRemote ? '远端不可达，仅还原未提交更改（未执行 pull）' : '无远端服务器，仅还原未提交更改（未执行 pull）');
    }

    // 指定分支：先还原（上方已执行）→ 尝试切换到该分支 → 再拉取
    const branch = String(node.branch || '').trim();
    if (branch) {
      const cur = String(runSync('git', ['rev-parse', '--abbrev-ref', 'HEAD'], abs).stdout || '').trim();
      if (cur !== branch) {
        let co = await runVcsProc(node, index, `切换到分支 ${branch}`, 'git', ['checkout', branch], abs, timeoutMs);
        if (vcs && vcs.cancel) return vcsNodeResult(node, index, name, type, 'cancelled', false, '已取消');
        // 本地没有该分支：若远端在线且有该分支，先 fetch 再基于远端创建本地跟踪分支
        if (!co.ok && rem.online && rem.remoteName) {
          pushVcsLine(`（本地不存在分支 ${branch}，尝试从远端 ${rem.remoteName} 检出跟踪分支）\n`);
          await runVcsProc(node, index, `拉取远端分支 ${branch}`, 'git', ['fetch', rem.remoteName, branch], abs, timeoutMs);
          co = await runVcsProc(node, index, `从远端创建并切换到分支 ${branch}`, 'git', ['checkout', '-b', branch, '--track', rem.remoteName + '/' + branch], abs, timeoutMs);
          if (vcs && vcs.cancel) return vcsNodeResult(node, index, name, type, 'cancelled', false, '已取消');
        }
        if (!co.ok) return vcsNodeResult(node, index, name, type, 'fail', false, '切换分支失败（已尝试本地检出与基于远端创建，本地与远端均无该分支？）');
      }
    }
    let u = await runVcsProc(node, index, '拉取最新代码', 'git', ['pull'], abs, timeoutMs);
    if (vcs && vcs.cancel) return vcsNodeResult(node, index, name, type, 'cancelled', false, '已取消');
    // 分支无上游（upstream）时 pull 会失败：退回按远端 + 分支名拉取
    if (!u.ok && branch && rem.online && rem.remoteName) {
      u = await runVcsProc(node, index, `拉取远端 ${rem.remoteName}/${branch}`, 'git', ['pull', rem.remoteName, branch], abs, timeoutMs);
      if (vcs && vcs.cancel) return vcsNodeResult(node, index, name, type, 'cancelled', false, '已取消');
    }
    if (!u.ok) return vcsNodeResult(node, index, name, type, 'fail', false, '更新失败（退出码 ' + u.exitCode + '）');
    return vcsNodeResult(node, index, name, type, 'ok', true, branch ? ('更新成功（分支 ' + branch + '）') : '更新成功');
  }

  // SVN：--non-interactive 避免凭据 / 交互提示挂起
  const info = svnInfo(abs);
  pushVcsLine(info.remoteOnline
    ? `（SVN 服务器在线：${info.branch || ''}）\n`
    : ('（SVN 服务器不可达，仅执行还原，跳过 update' + (info.error ? '：' + info.error : '') + '）\n'));

  // 还原未提交更改：优先使用节点自定义还原指令，否则默认普通还原（svn revert -R .）
  const customRevert = nodeRevertCommand(node);
  if (customRevert) {
    const rr = await runVcsShell(node, index, '自定义还原指令', customRevert, abs, timeoutMs);
    if (vcs && vcs.cancel) return vcsNodeResult(node, index, name, type, 'cancelled', false, '已取消');
    if (!rr.ok) return vcsNodeResult(node, index, name, type, 'fail', false, '自定义还原指令执行失败（退出码 ' + rr.exitCode + '）');
  } else {
    const rv = await runVcsProc(node, index, '还原未提交更改', 'svn', ['--non-interactive', 'revert', '-R', '.'], abs, timeoutMs);
    if (vcs && vcs.cancel) return vcsNodeResult(node, index, name, type, 'cancelled', false, '已取消');
    if (!rv.ok) return vcsNodeResult(node, index, name, type, 'fail', false, '还原未提交更改失败（退出码 ' + rv.exitCode + '）');
  }

  if (!info.remoteOnline) {
    return vcsNodeResult(node, index, name, type, 'partial', true, 'SVN 服务器不可达，仅还原未提交更改（未执行 update）');
  }

  // 指定分支：svn switch 到分支 URL 后再 update
  const branch = String(node.branch || '').trim();
  if (branch) {
    const targetUrl = resolveSvnBranchUrl(branch, info.root || null, info.branch || null);
    if (targetUrl && normalizeUrl(targetUrl) !== normalizeUrl(info.branch)) {
      const sw = await runVcsProc(node, index, `切换到分支 URL ${targetUrl}`, 'svn', ['--non-interactive', 'switch', targetUrl], abs, timeoutMs);
      if (vcs && vcs.cancel) return vcsNodeResult(node, index, name, type, 'cancelled', false, '已取消');
      if (!sw.ok) return vcsNodeResult(node, index, name, type, 'fail', false, '切换分支失败（分支 URL 不存在？）');
    } else if (!targetUrl) {
      pushVcsLine('（无法解析 SVN 分支 URL，跳过切换，按当前分支更新）\n');
    }
  }
  const u = await runVcsProc(node, index, '更新到最新版本', 'svn', ['--non-interactive', 'update'], abs, timeoutMs);
  if (vcs && vcs.cancel) return vcsNodeResult(node, index, name, type, 'cancelled', false, '已取消');
  if (!u.ok) return vcsNodeResult(node, index, name, type, 'fail', false, '更新失败（退出码 ' + u.exitCode + '）');
  return vcsNodeResult(node, index, name, type, 'ok', true, branch ? '更新成功（已切换到指定分支）' : '更新成功');
}

function summarizeVcs(results) {
  const s = { total: 0, ok: 0, partial: 0, fail: 0, skipped: 0, cancelled: 0 };
  for (const r of Object.values(results || {})) {
    s.total++;
    if (r.status === 'skipped') s.skipped++;
    else if (r.status === 'cancelled') s.cancelled++;
    else if (r.status === 'partial') s.partial++;
    else if (r.ok) s.ok++;
    else s.fail++;
  }
  return s;
}

async function startVcsUpdate(req) {
  if (job && (job.state === 'running' || job.state === 'starting')) return { ok: false, error: '批量打包进行中，请先停止构建' };
  if (check && (check.state === 'running' || check.state === 'starting')) return { ok: false, error: '环境编译检测进行中，请先停止检测' };
  if (vcs && (vcs.state === 'running' || vcs.state === 'starting')) return { ok: false, error: '已有版本更新任务在运行' };

  let nodes = vcsConfig().nodes.slice();
  if (Array.isArray(req.ids) && req.ids.length) {
    const set = new Set(req.ids.map(String));
    nodes = nodes.filter(n => set.has(String(n.id)));
  }
  if (!nodes.length) return { ok: false, error: '没有可更新的版本管理节点（请先添加节点并填写路径）' };
  const timeoutMs = Math.max(30000, (Number(req.timeoutMinutes) || 30) * 60000);

  vcs = { state: 'starting', nodes, index: 0, lines: [], results: {}, child: null, cancel: false, end: null, startedAt: new Date().toISOString() };
  pushVcsLine(`\n════════ 版本管理更新启动：共 ${nodes.length} 个节点（更新前自动还原未提交更改） ════════\n`);
  broadcast({ type: 'vcs-start', count: nodes.length, ids: nodes.map(n => n.id) });

  const run = async () => {
    vcs.state = 'running';
    try {
      for (let i = 0; i < nodes.length; i++) {
        if (vcs.cancel) { broadcast({ type: 'vcs-end', ok: false, reason: 'cancelled' }); return; }
        vcs.index = i;
        const node = nodes[i];
        pushVcsLine(`\n════════ [${i + 1}/${nodes.length}] 节点：${node.name || node.id}（${node.path || '路径为空'}） ════════\n`);
        broadcast({ type: 'vcs-node-start', index: i, nodeId: node.id, name: node.name || node.id, path: node.path || '' });
        await updateVcsNode(node, i, timeoutMs);
        if (vcs.cancel) { broadcast({ type: 'vcs-end', ok: false, reason: 'cancelled' }); return; }
      }
      const s = summarizeVcs(vcs.results);
      pushVcsLine(`\n════════ 版本管理更新结束：更新成功 ${s.ok} ｜ 仅还原 ${s.partial} ｜ 失败 ${s.fail} ｜ 跳过 ${s.skipped} ════════\n`);
      broadcast({ type: 'vcs-end', ok: s.fail === 0 && s.cancelled === 0, summary: s });
    } catch (e) {
      console.error('[vcs] 异常:', e);
      try { broadcast({ type: 'vcs-end', ok: false, reason: 'error', message: e.message }); } catch { /* */ }
    } finally {
      vcs.state = 'done';
    }
  };

  run();
  return { ok: true };
}

function stopVcs() {
  if (!vcs || vcs.state === 'done') return { ok: false, error: '没有运行中的版本更新任务' };
  vcs.cancel = true;
  killProc(vcs.child);
  return { ok: true };
}

/* ── 打包前自动更新（构建钩子） ── */

/** 取指定分组下的全部节点（保序、去重） */
function nodeIdsOfGroups(groupIds) {
  const vc = vcsConfig();
  const gidSet = new Set((groupIds || []).map(String));
  const ids = new Set();
  for (const g of vc.groups) {
    if (gidSet.has(String(g.id))) for (const id of (g.nodeIds || [])) ids.add(String(id));
  }
  return vc.nodes.filter(n => ids.has(String(n.id)));
}

function groupNamesOf(groupIds) {
  const vc = vcsConfig();
  const gidSet = new Set((groupIds || []).map(String));
  return vc.groups.filter(g => gidSet.has(String(g.id))).map(g => g.name || g.id);
}

/**
 * 在批量构建任务内执行「构建前版本更新」：每个构建配置（队列中的 Profile）
 * 单独配置自己的版本更新组，构建该配置前执行其分组更新。
 * 复用现有 vcs 更新机制（revert → 分支切换 → 远端在线才 pull / update），
 * 事件带 preBuild=true 经 SSE 推送；结束后 vcs 全局状态还原为 null。
 * 返回 { continue } —— 任一节点失败或取消则该配置停止构建。
 */
async function runPreBuildVcsUpdate(nodes, groupNames, label, profileIndex) {
  const timeoutMs = 30 * 60000;
  const who = label || '构建前';
  vcs = { state: 'running', nodes, index: 0, lines: [], results: {}, child: null, cancel: false, end: null, preBuild: true, startedAt: new Date().toISOString() };
  broadcast({ type: 'notice', text: `[版本管理] ${who} 版本更新启动（分组：${groupNames.join('、') || '—'}，共 ${nodes.length} 个节点）` });
  pushVcsLine(`\n════════ 【构建前版本更新】${who} ｜ 分组：${groupNames.join('、') || '—'} ｜ 节点 ${nodes.length} 个 ════════\n`);
  broadcast({ type: 'vcs-start', count: nodes.length, ids: nodes.map(n => n.id), preBuild: true, groupNames, label: who, profileIndex });
  try {
    for (let i = 0; i < nodes.length; i++) {
      if (vcs.cancel || (job && job.cancel)) {
        broadcast({ type: 'vcs-end', preBuild: true, ok: false, reason: 'cancelled' });
        return { continue: false, reason: 'cancelled' };
      }
      vcs.index = i;
      const node = nodes[i];
      pushVcsLine(`\n════════ [${i + 1}/${nodes.length}] 节点：${node.name || node.id}（${node.path || '路径为空'}） ════════\n`);
      broadcast({ type: 'vcs-node-start', index: i, nodeId: node.id, name: node.name || node.id, path: node.path || '', preBuild: true });
      await updateVcsNode(node, i, timeoutMs);
      if (vcs.cancel || (job && job.cancel)) {
        broadcast({ type: 'vcs-end', preBuild: true, ok: false, reason: 'cancelled' });
        return { continue: false, reason: 'cancelled' };
      }
    }
    const s = summarizeVcs(vcs.results);
    pushVcsLine(`\n════════ ${who} 版本更新结束：更新成功 ${s.ok} ｜ 仅还原 ${s.partial} ｜ 失败 ${s.fail} ｜ 跳过 ${s.skipped} ════════\n`);
    const ok = s.fail === 0 && s.cancelled === 0;
    broadcast({ type: 'vcs-end', preBuild: true, ok, summary: s });
    broadcast({ type: 'notice', text: ok
      ? `[版本管理] ${who} 版本更新完成：成功 ${s.ok}，仅还原 ${s.partial}，跳过 ${s.skipped}，继续构建`
      : `[版本管理] ${who} 版本更新存在失败（失败 ${s.fail}），停止构建` });
    return { continue: ok, reason: ok ? null : 'vcsPreFail', message: null };
  } catch (e) {
    console.error('[vcs] 构建前更新异常:', e);
    broadcast({ type: 'vcs-end', preBuild: true, ok: false, reason: 'error', message: e.message });
    return { continue: false, reason: 'error', message: e.message };
  } finally {
    vcs = null; // 构建前更新不占用独立 vcs 状态
  }
}

// ─────────────────────────── HTTP 服务 ───────────────────────────

function readBody(req) {
  return new Promise((resolve, reject) => {
    let buf = '';
    req.on('data', c => { buf += c; if (buf.length > 5e6) reject(new Error('body too large')); });
    req.on('end', () => {
      if (!buf) return resolve(null);
      try {
        buf = buf.replace(/^\uFEFF/, '');
        resolve(JSON.parse(buf));
      } catch (e) { reject(e); }
    });
    req.on('error', reject);
  });
}

function sendJson(res, code, data) {
  res.writeHead(code, { 'Content-Type': 'application/json; charset=utf-8' });
  res.end(JSON.stringify(data));
}

const MIME = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.json': 'application/json; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png' };

const server = http.createServer(async (req, res) => {
  try {
    const u = new URL(req.url, `http://${req.headers.host || 'localhost'}`);
    const p = u.pathname;

    if (req.method === 'GET' && (p === '/' || !p.startsWith('/api/'))) {
      const target = p === '/' ? '/index.html' : p;
      const file = path.normalize(path.join(PUBLIC, target));
      if (!file.startsWith(PUBLIC)) { sendJson(res, 403, { error: 'forbidden' }); return; }
      if (!fs.existsSync(file) || !fs.statSync(file).isFile()) { sendJson(res, 404, { error: 'not found' }); return; }
      res.writeHead(200, { 'Content-Type': MIME[path.extname(file)] || 'application/octet-stream', 'Cache-Control': 'no-store' });
      fs.createReadStream(file).pipe(res);
      return;
    }

    // ── API ──
    if (req.method === 'GET' && p === '/api/status') {
      sendJson(res, 200, {
        config,
        unityExes: findUnityExes(),
        cli: findUnityCli(),
        outputs: scanOutputs(resolveOutBase()),
        job: job ? { state: job.state, index: job.index, count: job.queue.length, cancel: job.cancel } : null,
        check: check ? {
          state: check.state, index: check.index, count: check.targets.length,
          targets: check.targets, results: check.results, end: check.end || null, runDir: check.runDir,
        } : null,
        vcs: vcs ? {
          state: vcs.state, index: vcs.index, count: vcs.nodes.length,
          results: vcs.results, end: vcs.end || null,
        } : null,
      });
      return;
    }

    if ((req.method === 'GET' || req.method === 'POST') && p === '/api/outputs') {
      sendJson(res, 200, { ok: true, outputs: scanOutputs(resolveOutBase()) });
      return;
    }

    if (req.method === 'POST' && p === '/api/config') {
      const body = await readBody(req);
      if (!body) { sendJson(res, 400, { error: 'bad body' }); return; }
      config = { ...config, ...body };
      // 版本更新组已改为按构建配置（Profile）单独配置，清理旧的全局配置键
      delete config.vcsBeforeBuild;
      saveConfig();
      sendJson(res, 200, { ok: true, config });
      return;
    }

    if (req.method === 'POST' && p === '/api/scan') {
      const body = await readBody(req) || {};
      const projectAbs = resolveProject(body.projectPath);
      if (!isValidProject(projectAbs)) { sendJson(res, 400, { error: '项目路径无效，请检查绝对路径或相对路径（相对路径基于本工具目录）' }); return; }
      const scan = scanProfiles(projectAbs, body.profileDir);
      sendJson(res, 200, {
        ok: true,
        projectAbs,
        unityVersion: readUnityVersion(projectAbs),
        profiles: scan.found,
        ignored: scan.ignored,
      });
      return;
    }

    if (req.method === 'GET' && p === '/api/unity-detect') {
      sendJson(res, 200, { exes: findUnityExes(), cli: findUnityCli() });
      return;
    }

    if (req.method === 'POST' && p === '/api/cli-probe') {
      sendJson(res, 200, probeWithUnityCli());
      return;
    }

    if (req.method === 'POST' && p === '/api/cli-open') {
      const body = await readBody(req) || {};
      const projectAbs = resolveProject(body.projectPath);
      const cli = findUnityCli();
      if (!cli) { sendJson(res, 400, { error: '未找到 unity CLI（%LOCALAPPDATA%\\Unity\\bin\\unity.exe 或 PATH）' }); return; }
      if (!isValidProject(projectAbs)) { sendJson(res, 400, { error: '项目路径无效' }); return; }
      const child = spawn(cli, ['open', projectAbs], { cwd: ROOT, windowsHide: true, stdio: 'ignore', detached: true });
      child.unref();
      sendJson(res, 200, { ok: true, cli, projectAbs });
      return;
    }

    if (req.method === 'POST' && p === '/api/build/preview') {
      const body = await readBody(req) || {};
      const projectAbs = resolveProject(body.projectPath);
      const engine = body.engine || {};
      const profiles = (body.profiles || []).filter(x => x && x.assetPath);
      const outBase = body.outputBase ? (path.isAbsolute(body.outputBase) ? body.outputBase : path.join(projectAbs || '', body.outputBase)) : path.join(projectAbs || '', 'Builds');
      const buildNumberRaw = Number(body.buildNumber);
      const buildNumberAuto = !Number.isFinite(buildNumberRaw) || buildNumberRaw === -1;
      const items = profiles.map(pr => {
        const sign = Object.assign({}, body.sign || {}, pr.sign || {});
        const customDir = pr.customBuildDir || sign.customBuildDir || null;
        const resolvedAndroidKind = String(pr.androidBuildKind || sign.androidBuildKind || body.androidBuildKind || 'apk').toLowerCase();
        const ctx = makeOutputs(pr, outBase, customDir, resolvedAndroidKind);
        ctx.projectAbs = projectAbs;
        ctx.sign = sign;
        ctx.dev = pr.dev != null ? pr.dev : (sign.dev != null ? sign.dev : !!body.dev);
        ctx.buildAddressables = pr.buildAddressables != null ? pr.buildAddressables : (sign.buildAddressables != null ? sign.buildAddressables : !!body.buildAddressables);
        ctx.addressablesMethod = pr.addressablesMethod || sign.addressablesMethod || body.addressablesMethod || null;
        ctx.androidBuildKind = resolvedAndroidKind;
        ctx.nameTemplate = pr.nameTemplate || sign.nameTemplate || body.artifactNameTemplate || cfg('artifactNameTemplate', '');
        ctx.autoZip = pr.autoZip != null ? pr.autoZip : (sign.autoZip != null ? sign.autoZip : (body.autoZip != null ? body.autoZip : cfg('defaultAutoZip', true)));

        const buildNo = buildNumberAuto ? ((Number(config.lastVersionCode) || 0) + 1) : buildNumberRaw;
        if (pr.target === 13) ctx.versionCode = buildNo;
        ctx.buildNo = buildNo;
        const cmd = composeCommand(engine, pr, ctx);
        const previewName = formatArtifactName(ctx.nameTemplate, {
          Platform: pr.targetName,
          Target: pr.targetName,
          Profile: pr.name,
          Product: pr.productName || pr.name,
          Version: pr.bundleVersion || '1.0',
          VersionCode: buildNo,
          Time: '<时间戳>',
          Date: '<日期>',
          Dev: ctx.dev,
        });

        return {
          name: pr.name,
          target: pr.targetName,
          output: ctx.output,
          logFile: ctx.logFile,
          command: cmd.display,
          versionCode: ctx.versionCode || null,
          buildNo: buildNo || null,
          dev: ctx.dev,
          buildAddressables: ctx.buildAddressables,
          androidBuildKind: ctx.androidBuildKind,
          autoZip: ctx.autoZip,
          customBuildDir: customDir || null,
          previewName,
          archiveDir: path.join(outBase, (body.successDir || '').trim() || cfg('successDir', '构建成功'), pr.name + '-<时间戳>'),
          // 该构建配置单独配置的版本更新组
          vcsBeforeBuild: (() => {
            const gids = Array.isArray(pr.vcsGroupIds) ? pr.vcsGroupIds.map(String) : [];
            const pn = gids.length ? nodeIdsOfGroups(gids) : [];
            return {
              enabled: pn.length > 0,
              groups: groupNamesOf(gids),
              nodeCount: pn.length,
              nodeNames: pn.map(n => n.name || n.id),
            };
          })(),
        };
      });
      sendJson(res, 200, { ok: true, items, engine });
      return;
    }

    if (req.method === 'POST' && p === '/api/build/start') {
      const body = await readBody(req) || {};
      const r = await startJob(body);
      if (!r.ok) { sendJson(res, 400, r); return; }
      sendJson(res, 200, { ok: true });
      return;
    }

    if (req.method === 'POST' && p === '/api/build/stop') {
      sendJson(res, 200, stopJob());
      return;
    }

    // ── 环境编译检测 API ──
    if (req.method === 'POST' && p === '/api/check/start') {
      const body = await readBody(req) || {};
      const r = await startCheck(body);
      if (!r.ok) { sendJson(res, 400, r); return; }
      sendJson(res, 200, { ok: true, runDir: r.runDir });
      return;
    }

    if (req.method === 'POST' && p === '/api/check/stop') {
      sendJson(res, 200, stopCheck());
      return;
    }

    if (req.method === 'POST' && p === '/api/check/preview') {
      const body = await readBody(req) || {};
      const projectAbs = resolveProject(body.projectPath);
      const engine = body.engine || {};
      const raw = (body.targets || []).filter(t => ALLOWED_TARGETS.has(Number(t))).map(Number);
      const targets = [...new Set(raw)];
      const timeoutMinutes = Math.max(1, Number(body.timeoutMinutes) || 20);
      const runDir = path.join(CHECK_RESULTS_DIR, 'check-<时间戳>');
      const items = targets.map(t => {
        const files = { json: path.join(runDir, 'target-' + t + '.json'), log: path.join(runDir, 'target-' + t + '.log') };
        const cmd = composeCheckCommand(engine, projectAbs || '<项目路径>', t, files, timeoutMinutes);
        return { target: t, name: TARGETS[t] || ('BuildTarget ' + t), command: cmd.display, logFile: files.log };
      });
      sendJson(res, 200, { ok: true, items, targets, timeoutMinutes });
      return;
    }

    // ── 版本管理 API ──
    if (req.method === 'GET' && p === '/api/vcs/state') {
      sendJson(res, 200, { ok: true, vcs: vcsConfig() });
      return;
    }

    if (req.method === 'POST' && p === '/api/vcs/save') {
      const body = await readBody(req) || {};
      saveVcsConfig({ nodes: body.nodes, groups: body.groups });
      sendJson(res, 200, { ok: true });
      return;
    }

    if (req.method === 'POST' && p === '/api/vcs/probe') {
      const body = await readBody(req) || {};
      let nodes = vcsConfig().nodes.slice();
      if (Array.isArray(body.nodes) && body.nodes.length) {
        // 临时探测（未保存的节点，用于添加节点弹窗内「检测类型」）
        nodes = body.nodes.map((n, i) => ({ id: 'adhoc' + i, name: n.name || ('adhoc' + i), path: n.path, branch: n.branch, revert: n.revert, type: n.type || 'auto' }));
      } else if (Array.isArray(body.ids) && body.ids.length) {
        const set = new Set(body.ids.map(String));
        nodes = nodes.filter(n => set.has(String(n.id)));
      }
      sendJson(res, 200, { ok: true, results: nodes.map(probeNode) });
      return;
    }

    if (req.method === 'POST' && p === '/api/vcs/update') {
      const body = await readBody(req) || {};
      const r = await startVcsUpdate(body);
      if (!r.ok) { sendJson(res, 400, r); return; }
      sendJson(res, 200, { ok: true });
      return;
    }

    if (req.method === 'POST' && p === '/api/vcs/stop') {
      sendJson(res, 200, stopVcs());
      return;
    }

    if (req.method === 'POST' && p === '/api/ai/analyze') {
      const body = await readBody(req) || {};
      const r = await aiAnalyze(body);
      if (!r.ok) { sendJson(res, 400, r); return; }
      sendJson(res, 200, r);
      return;
    }

    // 打开构建产物所在文件夹（支持直接打开目录、产物文件所在目录或输出根目录）
    if (req.method === 'POST' && p === '/api/open-folder') {
      const body = await readBody(req) || {};
      const target = (body.path || '').trim();
      try {
        const projectAbs = resolveProject(cfg('projectPath', '../..')) || path.resolve(ROOT, '..', '..');
        const outBase = resolveOutBase() || path.join(projectAbs, 'Builds');
        
        let abs;
        if (!target || target === 'Builds' || target === cfg('outputBase', 'Builds')) {
          abs = outBase;
        } else if (path.isAbsolute(target)) {
          abs = path.normalize(target);
        } else {
          const inProj = path.join(projectAbs, target);
          const inOut = path.join(outBase, target);
          const inTool = path.resolve(ROOT, target);
          if (fs.existsSync(inProj)) abs = inProj;
          else if (fs.existsSync(inOut)) abs = inOut;
          else if (fs.existsSync(inTool)) abs = inTool;
          else abs = inProj;
        }

        let folderToOpen = abs;
        if (!fs.existsSync(folderToOpen)) {
          try {
            fs.mkdirSync(folderToOpen, { recursive: true });
          } catch {
            const parent = path.dirname(folderToOpen);
            if (fs.existsSync(parent)) folderToOpen = parent;
            else if (fs.existsSync(outBase)) folderToOpen = outBase;
            else if (fs.existsSync(projectAbs)) folderToOpen = projectAbs;
          }
        }

        if (fs.existsSync(folderToOpen)) {
          const st = fs.statSync(folderToOpen);
          if (!st.isDirectory()) {
            folderToOpen = path.dirname(folderToOpen);
          }
        }

        if (process.platform === 'win32') {
          const ch = spawn('explorer.exe', [folderToOpen], { stdio: 'ignore', detached: true, windowsHide: false });
          ch.unref();
        } else if (process.platform === 'darwin') {
          spawn('open', [folderToOpen], { stdio: 'ignore', detached: true }).unref();
        } else {
          spawn('xdg-open', [folderToOpen], { stdio: 'ignore', detached: true }).unref();
        }
        sendJson(res, 200, { ok: true, opened: folderToOpen });
      } catch (e) {
        sendJson(res, 400, { error: e.message });
      }
      return;
    }

    // ── SSE ──
    if (req.method === 'GET' && p === '/api/build/events') {
      res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache', Connection: 'keep-alive', 'X-Accel-Buffering': 'no' });
      res.write('retry: 3000\n\n');
      if (job) {
        res.write('data: ' + JSON.stringify({
          type: 'hello',
          job: {
            state: job.state, index: job.index, count: job.queue.length,
            queue: job.queue.map(p => ({ name: p.name, targetName: p.targetName, assetPath: p.assetPath })),
            progress: job.progress || {},
            end: job.end || null,
          },
        }) + '\n\n');
        const tail = job.lines.slice(-300);
        for (const l of tail) res.write('data: ' + JSON.stringify({ type: 'line', index: l.i, text: l.t }) + '\n\n');
      } else {
        res.write('data: ' + JSON.stringify({ type: 'hello', job: null }) + '\n\n');
      }
      if (check) {
        res.write('data: ' + JSON.stringify({
          type: 'check-hello',
          check: {
            state: check.state, index: check.index, count: check.targets.length,
            targets: check.targets, results: check.results || {}, end: check.end || null, runDir: check.runDir,
          },
        }) + '\n\n');
        const ctail = check.lines.slice(-300);
        for (const l of ctail) res.write('data: ' + JSON.stringify({ type: 'check-line', text: l }) + '\n\n');
      } else {
        res.write('data: ' + JSON.stringify({ type: 'check-hello', check: null }) + '\n\n');
      }
      if (vcs) {
        res.write('data: ' + JSON.stringify({
          type: 'vcs-hello',
          vcs: {
            state: vcs.state, index: vcs.index, count: vcs.nodes.length,
            results: vcs.results || {}, end: vcs.end || null,
          },
        }) + '\n\n');
        const vtail = vcs.lines.slice(-300);
        for (const l of vtail) res.write('data: ' + JSON.stringify({ type: 'vcs-line', text: l }) + '\n\n');
      } else {
        res.write('data: ' + JSON.stringify({ type: 'vcs-hello', vcs: null }) + '\n\n');
      }
      sseClients.add(res);
      const iv = setInterval(() => { try { res.write(': ping\n\n'); } catch { /* */ } }, 20000);
      req.on('close', () => { clearInterval(iv); sseClients.delete(res); });
      return;
    }

    sendJson(res, 404, { error: 'not found: ' + p });
  } catch (e) {
    console.error('[http]', e);
    try { sendJson(res, 500, { error: e.message }); } catch { /* */ }
  }
});

if (require.main === module) {
  server.listen(PORT, HOST, () => {
    console.log('====================================================');
    console.log('  TBuildTool · 本地批量打包工具');
    console.log(`  地址: http://${HOST}:${PORT}`);
    console.log('  关闭窗口即停止服务');
    console.log('====================================================');
    if (AUTO_OPEN) {
      setTimeout(() => {
        try {
          const url = `http://${HOST}:${PORT}`;
          if (process.platform === 'win32') spawn('cmd', ['/c', 'start', '', url], { stdio: 'ignore', detached: true, windowsHide: true }).unref();
          else if (process.platform === 'darwin') spawn('open', [url], { stdio: 'ignore', detached: true }).unref();
          else spawn('xdg-open', [url], { stdio: 'ignore', detached: true }).unref();
        } catch { /* 忽略 */ }
      }, 600);
    }
  });
}

module.exports = { writeZip, scanOutputs, makeOutputs, resolveOutBase, finalizeArtifact, crc32, findArtifact, readBuildInfo, formatArtifactName };

'use strict';
/**
 * TBuildTool · MiSide 壁纸 本地批量打包工具 —— 前端交互逻辑
 */

const $ = id => document.getElementById(id);
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

// 环境线（BuildTarget → 展示信息），与后端 TARGETS / ALLOWED_TARGETS 一致
const ENV_LINES = [
  { target: 19, name: 'Windows (x64)', badge: 'win',   icon: '🪟' },
  { target: 5,  name: 'Windows (x86)', badge: 'win',   icon: '🪟' },
  { target: 13, name: 'Android',       badge: 'android', icon: '🤖' },
  { target: 9,  name: 'iOS',           badge: 'ios',   icon: '🍎' },
  { target: 4,  name: 'macOS',         badge: 'mac',   icon: '🍏' },
];

const state = {
  config: {},
  profiles: [],        // 扫描结果（全部）
  filter: 'all',
  queue: [],           // 选中的 profile 引用（顺序即打包顺序）
  profileConfigs: {},  // assetPath -> { customBuildDir, nameTemplate, dev, buildAddressables, addressablesMethod, keystoreName, keyaliasName, keystorePass, keyaliasPass, remember }
  job: null,           // 最近一次 SSE 状态
  jobProgress: null,
  outputs: [],         // 构建产物列表
  cfgTarget: null,     // Profile 配置弹窗当前编辑的 assetPath
  rawLogs: '',         // 原始日志内容，供搜索过滤与复制
  currentTab: 'home',
  checkTargets: [19, 13], // 环境编译检测：选中的环境线
  checkResults: {},       // target -> { target, name, status, ms, message, ok }
  checkRawLogs: '',       // 环境检测日志
  checkJob: null,         // 最近一次环境检测 SSE 状态
  checkRunDir: null,      // 本次检测结果目录
  vcsNodes: [],           // 版本管理节点：[{ id, name, path, type }]（type: auto/git/svn）
  vcsGroups: [],          // 版本管理分组：[{ id, name, nodeIds }]
  vcsNodeStates: {},      // nodeId -> { status, type, branch, revision, dirty, dirtyCount, message }
  vcsJob: null,           // 最近一次版本更新 SSE 状态
  vcsRawLogs: '',         // 版本管理日志
  vcsEditId: null,        // 节点弹窗正在编辑的节点 id（null = 新增）
  vcsDragId: null,        // 正在拖动的节点 id
};

/* ─────────────── 标签页切换 Tab Navigation ─────────────── */

function switchTab(tabId) {
  state.currentTab = tabId;
  document.querySelectorAll('.sidebar-nav .nav-item').forEach(b => {
    b.classList.toggle('active', b.dataset.tab === tabId);
  });
  document.querySelectorAll('.tab-pane').forEach(p => {
    p.classList.toggle('active', p.id === `tab-pane-${tabId}`);
  });
}

/* ─────────────── Toast 提示通知 ─────────────── */

function showToast(text, type = 'info') {
  const container = $('toastContainer');
  if (!container) return;
  const t = document.createElement('div');
  t.className = `toast ${type}`;
  t.innerHTML = `<span>${esc(text)}</span>`;
  container.appendChild(t);
  setTimeout(() => {
    t.style.opacity = '0';
    t.style.transform = 'translateY(10px)';
    t.style.transition = 'all 0.25s ease';
    setTimeout(() => t.remove(), 250);
  }, 2500);
}

/* ─────────────── 剪贴板复制辅助 ─────────────── */

async function copyToClipboard(text, successMsg = '已复制到剪贴板') {
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text);
    } else {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.style.position = 'fixed';
      ta.style.left = '-9999px';
      document.body.appendChild(ta);
      ta.focus();
      ta.select();
      document.execCommand('copy');
      ta.remove();
    }
    showToast(successMsg, 'ok');
  } catch (e) {
    showToast('复制失败: ' + e.message, 'err');
  }
}

/* ─────────────── 通用 API 请求 ─────────────── */

async function api(path, body) {
  const opt = { method: 'GET' };
  if (body !== undefined) {
    opt.method = 'POST';
    opt.headers = { 'Content-Type': 'application/json' };
    opt.body = JSON.stringify(body);
  }
  const r = await fetch(path, opt);
  const ct = r.headers.get('content-type') || '';
  if (!r.ok) {
    let msg = 'HTTP ' + r.status;
    try { const j = await r.json(); msg = j.error || j.reason || msg; } catch { /* */ }
    throw new Error(msg);
  }
  return ct.includes('json') ? r.json() : r.text();
}

function setInfo(text, cls) {
  const el = $('projectInfo');
  if (!el) return;
  if (!text) {
    el.textContent = '';
    el.className = 'info-banner hidden';
    return;
  }
  el.textContent = text;
  el.className = 'info-banner' + (cls ? ' ' + cls : '');
  el.classList.remove('hidden');
}

/* ─────────────── 配置读写与表单同步 ─────────────── */

let saveTimer = null;
function collectConfig() {
  const mode = document.querySelector('input[name="engineMode"]:checked')?.value || 'batchmode';
  const engine = {
    mode,
    unityExe: $('unityExe').value.trim(),
    nographics: $('nographics').checked,
    cliPath: $('cliPath').value.trim(),
    template: $('template').value,
  };
  return {
    projectPath: $('projectPath').value.trim(),
    profileDir: $('profileDir').value.trim(),
    engine,
    artifactNameTemplate: $('artifactNameTemplate').value.trim(),
    outputBase: $('outputBase').value.trim(),
    successDir: $('successDir').value.trim() || '构建成功',
    defaultAutoZip: $('defaultAutoZip').checked,
    defaultDevBuild: $('defaultDevBuild').checked,
    defaultBuildAddressables: $('defaultBuildAddressables').checked,
    stopOnError: $('stopOnError').checked,
    proxy: {
      enabled: $('proxyEnabled').checked,
      host: $('proxyHost').value.trim(),
      port: $('proxyPort').value.trim(),
    },
    ai: {
      enabled: $('aiEnabled').checked,
      baseUrl: $('aiBaseUrl').value.trim(),
      model: $('aiModel').value.trim(),
      apiKey: $('aiApiKey').value.trim(),
    },
    buildNumber: $('buildNumber').value.trim() || '-1',
    profileConfigs: state.profileConfigs,
    check: {
      projectPath: $('checkProjectPath')?.value.trim() ?? '',
      unityExe: $('checkUnityExe')?.value.trim() ?? '',
      targets: state.checkTargets,
      timeoutMinutes: Number($('checkTimeout')?.value) || 20,
      nographics: $('checkNographics')?.checked ?? true,
    },
    queue: state.queue.map(p => ({
      name: p.name, target: p.target, targetName: p.targetName,
      assetPath: p.assetPath, absPath: p.absPath,
      productName: p.productName, subtarget: p.subtarget, sceneCount: p.sceneCount,
      bundleVersion: p.bundleVersion != null ? p.bundleVersion : null,
    })),
  };
}

function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    state.config = collectConfig();
    try {
      await api('/api/config', state.config);
    } catch (e) {
      console.warn('save config:', e.message);
    }
  }, 500);
}

function fillForm(cfg) {
  if (!cfg) return;
  $('projectPath').value = cfg.projectPath || '';
  $('profileDir').value = cfg.profileDir || '';
  const e = cfg.engine || {};
  const checkedRadio = document.querySelector(`input[name="engineMode"][value="${e.mode || 'batchmode'}"]`);
  if (checkedRadio) checkedRadio.checked = true;
  $('unityExe').value = e.unityExe || '';
  $('nographics').checked = e.nographics !== false;
  $('cliPath').value = e.cliPath || '';
  $('template').value = e.template || '';
  $('artifactNameTemplate').value = cfg.artifactNameTemplate || '{Product}_{Platform}_v{Version}_b{VersionCode}_{Time}{Dev}';
  $('outputBase').value = cfg.outputBase || '';
  $('successDir').value = cfg.successDir || '构建成功';
  $('defaultAutoZip').checked = cfg.defaultAutoZip !== false;
  $('defaultDevBuild').checked = !!cfg.defaultDevBuild;
  $('defaultBuildAddressables').checked = !!cfg.defaultBuildAddressables;
  $('stopOnError').checked = cfg.stopOnError !== false;
  const px = cfg.proxy || {};
  $('proxyEnabled').checked = !!px.enabled;
  $('proxyHost').value = px.host || '127.0.0.1';
  $('proxyPort').value = px.port || '10808';
  const ai = cfg.ai || {};
  $('aiEnabled').checked = !!ai.enabled;
  $('aiBaseUrl').value = ai.baseUrl || 'https://api.deepseek.com';
  $('aiModel').value = ai.model || 'deepseek-chat';
  $('aiApiKey').value = ai.apiKey || '';
  $('buildNumber').value = cfg.buildNumber != null ? cfg.buildNumber : '-1';
  state.profileConfigs = (cfg.profileConfigs && typeof cfg.profileConfigs === 'object') ? cfg.profileConfigs : {};
  // 环境编译检测配置恢复
  const ck = cfg.check || {};
  $('checkProjectPath').value = ck.projectPath || cfg.projectPath || '';
  $('checkUnityExe').value = ck.unityExe || (cfg.engine && cfg.engine.unityExe) || '';
  $('checkTimeout').value = ck.timeoutMinutes || 20;
  $('checkNographics').checked = ck.nographics !== false;
  if (Array.isArray(ck.targets) && ck.targets.length) {
    state.checkTargets = ck.targets.filter(t => ENV_LINES.some(e => e.target === Number(t))).map(Number);
  }
  renderCheckEnvChips();
  if (Array.isArray(cfg.queue) && cfg.queue.length) {
    state.queue = cfg.queue;
    renderQueue();
  }
  toggleEnginePanels();
}

/* ─────────────── 引擎面板切换 ─────────────── */

function toggleEnginePanels() {
  const mode = document.querySelector('input[name="engineMode"]:checked')?.value || 'batchmode';
  $('panel-batchmode').classList.toggle('hidden', mode !== 'batchmode');
  $('panel-template').classList.toggle('hidden', mode !== 'template');
}

/* ─────────────── 扫描 Build Profile ─────────────── */

async function doScan() {
  const projectPath = $('projectPath').value.trim();
  if (!projectPath) {
    setInfo('请先填写 Unity 项目路径', 'err');
    showToast('请先填写 Unity 项目路径', 'err');
    return;
  }
  setInfo('正在扫描项目中的 Build Profile…');
  $('btnScan').disabled = true;
  try {
    const r = await api('/api/scan', { projectPath, profileDir: $('profileDir').value.trim() });
    state.profiles = r.profiles || [];
    state.filter = 'all';
    const ver = r.unityVersion ? r.unityVersion : '（未识别）';
    const isU6 = r.unityVersion && r.unityVersion.startsWith('6000');
    setInfo(
      `✓ 项目路径: ${r.projectAbs}\n✓ Unity 版本: ${ver}${isU6 ? ' (Unity 6)' : '  ⚠ 非 6000.x，可能不是 Unity 6'} | 发现有效 Profile: ${r.profiles.length} 个`,
      isU6 ? 'ok' : ''
    );
    showToast(`扫描成功，发现 ${r.profiles.length} 个 Profile`, 'ok');
    const ig = (r.ignored || []);
    $('ignoredInfo').textContent = ig.length
      ? `已忽略目标不在支持范围内的 Profile：${ig.map(i => `${i.name}(${i.targetName})`).join('、')}`
      : '';
    renderProfiles();
  } catch (e) {
    setInfo('扫描失败：' + e.message, 'err');
    showToast('扫描失败：' + e.message, 'err');
  } finally {
    $('btnScan').disabled = false;
  }
}

function renderProfiles() {
  const tb = $('profileBody');
  if (!tb) return;
  const list = state.profiles.filter(p => {
    if (state.filter === 'all') return true;
    if (state.filter === 'android') return p.target === 13;
    if (state.filter === 'win') return p.target === 19 || p.target === 5;
    if (state.filter === 'ios') return p.target === 9;
    return true;
  });

  if (!list.length) {
    tb.innerHTML = `<tr><td colspan="4" class="empty-state">没有符合筛选条件的 Profile</td></tr>`;
    return;
  }

  tb.innerHTML = list.map(p => {
    const inQueue = state.queue.some(q => q.assetPath === p.assetPath);
    const isAndroid = p.target === 13;
    const isIOS = p.target === 9;
    const badgeClass = isAndroid ? 'android' : isIOS ? 'ios' : 'win';
    const targetLabel = isAndroid ? 'Android' : isIOS ? 'iOS' : 'Windows';
    return `<tr>
      <td class="name">
        <div>${esc(p.name)}</div>
        <div class="mono">${esc(p.assetPath)}</div>
      </td>
      <td>
        <span class="badge ${badgeClass}">${targetLabel}</span>
      </td>
      <td style="font-family: var(--font-mono); color: var(--text-muted);">${p.sceneCount}</td>
      <td style="text-align: right;">
        ${inQueue
          ? `<button class="btn btn-ghost btn-xs" disabled>已在队列</button>`
          : `<button class="btn btn-primary btn-xs" data-add="${esc(p.assetPath)}">＋ 添加</button>`}
      </td>
    </tr>`;
  }).join('');

  tb.querySelectorAll('button[data-add]').forEach(b => {
    b.onclick = () => {
      const p = state.profiles.find(x => x.assetPath === b.dataset.add);
      if (p) addToQueue(p);
    };
  });
}

/* ─────────────── 构建队列操作 ─────────────── */

function addToQueue(p) {
  if (!p || state.queue.some(q => q.assetPath === p.assetPath)) return;
  state.queue.push(p);
  renderQueue();
  renderProfiles();
  scheduleSave();
  showToast(`已添加「${p.name}」到队列`, 'ok');
}

function removeFromQueue(assetPath) {
  state.queue = state.queue.filter(q => q.assetPath !== assetPath);
  renderQueue();
  renderProfiles();
  scheduleSave();
}

function moveQueue(idx, dir) {
  const j = idx + dir;
  if (j < 0 || j >= state.queue.length) return;
  const [it] = state.queue.splice(idx, 1);
  state.queue.splice(j, 0, it);
  renderQueue();
  scheduleSave();
}

function renderQueue() {
  const ul = $('queueList');
  const count = state.queue.length;
  if ($('queueCount')) $('queueCount').textContent = `共 ${count} 个`;
  if ($('sidebarQueueCount')) $('sidebarQueueCount').textContent = String(count);
  if ($('homeQueueSummary')) $('homeQueueSummary').textContent = count ? `当前构建队列：${count} 个 Profile（按序执行）` : '当前队列为空，请在「Profile 与排期」中添加';

  if (!ul) return;
  if (!count) {
    ul.innerHTML = `<li class="empty-state">队列为空，请从左侧添加 Profile</li>`;
  } else {
    ul.innerHTML = state.queue.map((p, i) => {
      const isAndroid = p.target === 13;
      const isIOS = p.target === 9;
      const badgeClass = isAndroid ? 'android' : isIOS ? 'ios' : 'win';
      const targetLabel = isAndroid ? 'Android' : isIOS ? 'iOS' : 'Win';
      const cfg = state.profileConfigs[p.assetPath] || {};
      const isDev = cfg.dev != null ? cfg.dev : $('defaultDevBuild')?.checked;
      const isAddr = cfg.buildAddressables != null ? cfg.buildAddressables : $('defaultBuildAddressables')?.checked;
      const isAutoZip = cfg.autoZip != null ? cfg.autoZip : $('defaultAutoZip')?.checked;
      const isShared = !!cfg.customBuildDir;
      const isSigned = isAndroid && (cfg.keystoreName || cfg.keystorePass);

      const tags = [];
      if (isDev) tags.push(`<span class="tag-badge dev" title="DEV 调试包">DEV</span>`);
      if (isAddr) tags.push(`<span class="tag-badge addr" title="构建 Addressables">Addr</span>`);
      if (isAutoZip === false) tags.push(`<span class="tag-badge nozip" title="保留原始目录直接运行（不压缩为ZIP）">不压缩</span>`);
      if (isShared) tags.push(`<span class="tag-badge shared" title="共享/自定义缓存目录: ${esc(cfg.customBuildDir)}">缓存</span>`);
      if (isSigned) tags.push(`<span class="tag-badge signed" title="签名已配置">签名</span>`);

      return `<li>
        <span class="idx">${i + 1}</span>
        <span class="badge ${badgeClass}">${targetLabel}</span>
        <span class="nm">
          ${esc(p.name)}
          ${tags.join(' ')}
        </span>
        <button class="cfg-btn" data-cfg="${esc(p.assetPath)}" title="独立配置缓存目录、命名规则、Addressables 与签名">⚙ 配置</button>
        <button data-up="${i}" title="上移">↑</button>
        <button data-down="${i}" title="下移">↓</button>
        <button data-del="${esc(p.assetPath)}" title="从队列移除">×</button>
      </li>`;
    }).join('');

    ul.querySelectorAll('button[data-cfg]').forEach(b => b.onclick = () => openProfileCfg(b.dataset.cfg));
    ul.querySelectorAll('button[data-up]').forEach(b => b.onclick = () => moveQueue(+b.dataset.up, -1));
    ul.querySelectorAll('button[data-down]').forEach(b => b.onclick = () => moveQueue(+b.dataset.down, +1));
    ul.querySelectorAll('button[data-del]').forEach(b => b.onclick = () => removeFromQueue(b.dataset.del));
  }
}

/* ─────────────── Profile 独立配置弹窗 ─────────────── */

function openProfileCfg(assetPath) {
  const p = state.queue.find(q => q.assetPath === assetPath);
  if (!p) return;
  state.cfgTarget = assetPath;
  const cfg = state.profileConfigs[assetPath] || {};

  $('profileCfgTitle').textContent = `节点构建定制：${p.name}（${p.targetName}）`;
  $('pcCustomBuildDir').value = cfg.customBuildDir || '';
  $('pcNameTemplate').value = cfg.nameTemplate || '';
  $('pcAutoZip').checked = cfg.autoZip != null ? cfg.autoZip : $('defaultAutoZip').checked;
  $('pcDev').checked = cfg.dev != null ? cfg.dev : $('defaultDevBuild').checked;
  $('pcBuildAddressables').checked = cfg.buildAddressables != null ? cfg.buildAddressables : $('defaultBuildAddressables').checked;
  $('pcAddressablesMethod').value = cfg.addressablesMethod || '';

  // 区分平台专属卡片
  const isAndroid = p.target === 13;
  const isIOS = p.target === 9;
  const isWin = p.target === 19 || p.target === 5;

  $('pcAndroidBox').classList.toggle('hidden', !isAndroid);
  $('pcIosBox').classList.toggle('hidden', !isIOS);
  $('pcWinBox').classList.toggle('hidden', !isWin);

  if (isAndroid) {
    $('pcKeystoreName').value = cfg.keystoreName || p.keystoreName || '';
    $('pcAlias').value = cfg.keyaliasName || p.keyaliasName || 'key';
    $('pcPass').value = cfg.keystorePass || '';
    $('pcKeyPass').value = cfg.keyaliasPass || '';
    $('pcRemember').checked = !!cfg.remember;
    $('pcHint').textContent = '💡 安卓平台：Keystore 文件与别名已从 Profile 预填，填入密码即可。';
  } else if (isIOS) {
    $('pcHint').textContent = '💡 iOS 平台：导出 Xcode 工程后将自动打包压缩为 ZIP 归档。';
  } else {
    $('pcHint').textContent = '💡 Windows 平台：构建完成后将自动打包压缩为包含 exe 与 _Data 的 ZIP 归档。';
  }

  $('profileCfgModal').classList.remove('hidden');
}

function saveProfileCfg() {
  const assetPath = state.cfgTarget;
  if (!assetPath) return;
  const p = state.queue.find(q => q.assetPath === assetPath);
  const isAndroid = p && p.target === 13;

  const cfg = {
    customBuildDir: $('pcCustomBuildDir').value.trim(),
    nameTemplate: $('pcNameTemplate').value.trim(),
    autoZip: $('pcAutoZip').checked,
    dev: $('pcDev').checked,
    buildAddressables: $('pcBuildAddressables').checked,
    addressablesMethod: $('pcAddressablesMethod').value.trim(),
    keystoreName: isAndroid ? $('pcKeystoreName').value.trim() : '',
    keyaliasName: isAndroid ? $('pcAlias').value.trim() : '',
    keystorePass: (isAndroid && $('pcRemember').checked) ? $('pcPass').value : (isAndroid ? $('pcPass').value : ''),
    keyaliasPass: (isAndroid && $('pcRemember').checked) ? $('pcKeyPass').value : (isAndroid ? $('pcKeyPass').value : ''),
    remember: isAndroid ? $('pcRemember').checked : false,
  };

  state.profileConfigs[assetPath] = cfg;
  $('profileCfgModal').classList.add('hidden');
  renderQueue();
  scheduleSave();
  showToast('Profile 独立定制配置已保存', 'ok');
}

function cancelProfileCfg() {
  $('profileCfgModal').classList.add('hidden');
  state.cfgTarget = null;
}

/* ─────────────── 构建产物列表 ─────────────── */

function formatBytes(bytes) {
  if (!bytes || bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}

function renderOutputs() {
  const ul = $('outputList');
  if (!ul) return;
  if (!state.outputs.length) {
    ul.innerHTML = `<li class="empty-state">暂无构建产物（输出目录：${esc($('outputBase')?.value || 'Builds')}）</li>`;
    return;
  }
  ul.innerHTML = state.outputs.map((o, i) => {
    const fname = String(o.file || o.dir || '').split(/[\\/]/).pop();
    const isApk = o.kind === 'apk' || fname.toLowerCase().endsWith('.apk');
    const isZip = o.kind === 'zip' || fname.toLowerCase().endsWith('.zip');
    const isIOS = o.kind === 'ios' || (o.targetName || '').toLowerCase().includes('ios');
    const icon = isApk ? '🤖' : isIOS ? '🍎' : isZip ? '🗜️' : '📦';
    const meta = [
      o.targetName || '',
      o.version ? 'v' + o.version : '',
      o.versionCode != null ? 'b' + o.versionCode : '',
      o.dev ? 'DEV' : '',
      o.size ? formatBytes(o.size) : '',
      o.date || '',
    ].filter(Boolean).join(' · ');

    return `<li class="out-item" data-i="${i}" title="点击在资源管理器中打开所在文件夹">
      <span class="idx">${icon}</span>
      <span class="nm">${esc(o.name)}<small>${esc(fname)}${meta ? ' · ' + esc(meta) : ''}</small></span>
      <span class="out-icon-folder" title="打开所在文件夹">📁</span>
    </li>`;
  }).join('');

  ul.querySelectorAll('.out-item').forEach(el => {
    el.onclick = () => {
      const it = state.outputs[+el.dataset.i];
      if (it) openFolder(it.file || it.dir);
    };
  });
}

async function refreshOutputs() {
  try {
    const r = await api('/api/outputs');
    state.outputs = Array.isArray(r.outputs) ? r.outputs : [];
    renderOutputs();
    showToast(`产物列表已刷新，共 ${state.outputs.length} 个`, 'ok');
  } catch (e) {
    console.warn('outputs:', e.message);
  }
}

async function openFolder(p) {
  if (!p) return;
  try {
    const r = await api('/api/open-folder', { path: p });
    showToast(`已在资源管理器中打开: ${r.opened || ''}`, 'ok');
  } catch (e) {
    showToast('打开文件夹失败: ' + e.message, 'err');
  }
}

async function openOutputBaseFolder() {
  const outBase = $('outputBase')?.value.trim() || 'Builds';
  try {
    const r = await api('/api/open-folder', { path: outBase });
    showToast(`已打开输出目录: ${r.opened || outBase}`, 'ok');
  } catch (e) {
    showToast('打开输出目录失败: ' + e.message, 'err');
  }
}

/* ─────────────── 编辑器 / CLI 探测 ─────────────── */

async function doDetect() {
  try {
    const r = await api('/api/unity-detect');
    const exes = r.exes || [];
    if (!exes.length) {
      setInfo('未自动探测到 Unity 编辑器，请手动输入 Unity.exe 路径', 'err');
      showToast('未探测到编辑器', 'err');
      return;
    }
    const pre = exes.find(e => e.version.startsWith('6000')) || exes[0];
    $('unityExe').value = pre.path;
    fillUnitySelect(exes);
    setInfo(`已自动填入 Unity 路径: ${pre.path}`, 'ok');
    showToast(`已探测到 Unity ${pre.version}`, 'ok');
    scheduleSave();
  } catch (e) {
    setInfo('探测失败：' + e.message, 'err');
  }
}

function fillUnitySelect(exes) {
  const sel = $('unityExeSel');
  if (!sel) return;
  sel.innerHTML = '';
  if (!exes || !exes.length) {
    sel.innerHTML = `<option value="">（未探测到编辑器）</option>`;
    return;
  }
  for (const e of exes) {
    const opt = document.createElement('option');
    opt.value = e.path;
    opt.textContent = `${e.version} — ${e.path}`;
    sel.appendChild(opt);
  }
  sel.onchange = () => {
    $('unityExe').value = sel.value;
    scheduleSave();
  };
  const cur = $('unityExe').value.trim();
  if (cur && [...sel.options].some(o => o.value === cur)) sel.value = cur;
}

async function doCliProbe() {
  setInfo('正在调用 unity CLI（editors list）探测已安装编辑器…');
  try {
    const r = await api('/api/cli-probe');
    if (!r.cli) {
      setInfo('未找到 unity CLI（检查 %LOCALAPPDATA%\\Unity\\bin\\unity.exe 或 PATH）', 'err');
      showToast('未找到 unity CLI', 'err');
      return;
    }
    if (r.exes && r.exes.length) {
      fillUnitySelect(r.exes);
      const pre = r.exes.find(e => e.version.startsWith('6000')) || r.exes[0];
      $('unityExe').value = pre.path;
      setInfo(`CLI 探测成功（unity ${r.cli}）：已填入 ${pre.path}`, 'ok');
      showToast('CLI 探测成功', 'ok');
    } else {
      setInfo('CLI 可用，但未解析出编辑器列表\n' + (r.raw || r.error || '').slice(0, 400), 'err');
    }
    scheduleSave();
  } catch (e) {
    setInfo('CLI 探测失败：' + e.message, 'err');
  }
}

async function doCliOpen() {
  const projectPath = $('projectPath').value.trim();
  if (!projectPath) {
    setInfo('请先填写项目路径', 'err');
    return;
  }
  try {
    const r = await api('/api/cli-open', { projectPath });
    setInfo(`已通过 unity CLI 打开项目: ${r.projectAbs}`, 'ok');
    showToast('已通过 Unity CLI 打开项目', 'ok');
  } catch (e) {
    setInfo('打开失败：' + e.message, 'err');
  }
}

/* ─────────────── 构建流程与命令组合 ─────────────── */

function buildBody() {
  return {
    projectPath: $('projectPath').value.trim(),
    profiles: state.queue.map(p => {
      const cfg = state.profileConfigs[p.assetPath] || {};
      return Object.assign({}, p, {
        customBuildDir: cfg.customBuildDir || null,
        nameTemplate: cfg.nameTemplate || null,
        autoZip: cfg.autoZip != null ? cfg.autoZip : $('defaultAutoZip').checked,
        dev: cfg.dev != null ? cfg.dev : $('defaultDevBuild').checked,
        buildAddressables: cfg.buildAddressables != null ? cfg.buildAddressables : $('defaultBuildAddressables').checked,
        addressablesMethod: cfg.addressablesMethod || null,
        sign: cfg,
      });
    }),
    engine: collectConfig().engine,
    outputBase: $('outputBase').value.trim(),
    successDir: $('successDir').value.trim() || '构建成功',
    artifactNameTemplate: $('artifactNameTemplate').value.trim(),
    autoZip: $('defaultAutoZip').checked,
    dev: $('defaultDevBuild').checked,
    buildAddressables: $('defaultBuildAddressables').checked,
    stopOnError: $('stopOnError').checked,
    buildNumber: $('buildNumber').value.trim() || '-1',
    proxy: collectConfig().proxy,
  };
}

async function doPreview() {
  if (!state.queue.length) {
    showToast('队列为空，请先添加 Profile', 'err');
    switchTab('profiles');
    return;
  }
  try {
    const r = await api('/api/build/preview', buildBody());
    $('previewList').innerHTML = r.items.map(it => {
      const badges = [];
      if (it.dev) badges.push('[DEV]');
      if (it.buildAddressables) badges.push('[Addressables]');
      if (it.autoZip === false) badges.push('[直接运行/不压缩]');
      if (it.customBuildDir) badges.push(`[共享缓存: ${esc(it.customBuildDir)}]`);
      return `<div class="cmd-block">
        <span class="tag">▶ ${esc(it.name)} · ${esc(it.target)} ${badges.join(' ')}</span>
        <div class="hint">预计产物名: <strong>${esc(it.previewName)}</strong></div>
        <div class="hint">输出目标: ${esc(it.output)}</div>
        ${it.archiveDir ? `<div class="hint">归档目录: ${esc(it.archiveDir)}</div>` : ''}
        ${esc(it.command)}
      </div>`;
    }).join('') || '<div class="hint">（无预览命令）</div>';
    $('previewModal').classList.remove('hidden');
  } catch (e) {
    showToast('预览失败: ' + e.message, 'err');
  }
}

async function doStart() {
  if (!state.queue.length) {
    showToast('请先在队列中添加至少一个 Profile', 'err');
    switchTab('profiles');
    return;
  }
  if (!state.job || state.job.state !== 'running') {
    $('progressArea').classList.remove('hidden');
    buildProgressInit();
    state.rawLogs = '';
    $('logView').textContent = '';
  }
  $('btnStart').disabled = true;
  $('btnStop').disabled = false;
  showBanner('run', '正在启动批量构建流水线…');
  updateHeaderStatus('running', '批量打包中…');
  try {
    await api('/api/build/start', buildBody());
    showToast('已启动批量打包', 'ok');
  } catch (e) {
    showBanner('fail', '启动失败：' + e.message);
    updateHeaderStatus('fail', '启动失败');
    $('btnStart').disabled = false;
    $('btnStop').disabled = true;
    showToast('启动失败: ' + e.message, 'err');
  }
}

async function doStop() {
  try {
    await api('/api/build/stop');
    showToast('已请求终止构建', 'info');
  } catch (e) {
    console.warn(e);
  }
}

/* ─────────────── 环境编译检测 ─────────────── */

function checkBody() {
  return {
    projectPath: $('checkProjectPath').value.trim(),
    engine: {
      unityExe: $('checkUnityExe').value.trim(),
      nographics: $('checkNographics').checked,
    },
    targets: state.checkTargets,
    timeoutMinutes: Number($('checkTimeout').value) || 20,
  };
}

function renderCheckEnvChips() {
  const box = $('checkEnvChips');
  if (!box) return;
  box.innerHTML = ENV_LINES.map(e => {
    const on = state.checkTargets.includes(e.target);
    return `<button class="chip chip-${e.badge} ${on ? 'active' : ''}" data-env="${e.target}" title="环境线：${esc(e.name)}（BuildTarget ${e.target}）">${e.icon} ${esc(e.name)}</button>`;
  }).join('');
  box.querySelectorAll('button[data-env]').forEach(b => {
    b.onclick = () => {
      const t = Number(b.dataset.env);
      const i = state.checkTargets.indexOf(t);
      if (i >= 0) state.checkTargets.splice(i, 1);
      else state.checkTargets.push(t);
      renderCheckEnvChips();
      scheduleSave();
    };
  });
}

const CK_STATUS = {
  pending:     { label: '待检测', cls: 'pending' },
  running:     { label: '检测中', cls: 'running' },
  ok:          { label: '通过',   cls: 'ok' },
  fail:        { label: '失败',   cls: 'fail' },
  error:       { label: '异常',   cls: 'error' },
  unsupported: { label: '不支持', cls: 'unsupported' },
  timeout:     { label: '超时',   cls: 'timeout' },
  cancelled:   { label: '已取消', cls: 'cancelled' },
};

function renderCheckResults() {
  const ul = $('checkResultList');
  if (!ul) return;
  const targets = state.checkTargets;
  if (!targets.length) {
    ul.innerHTML = `<li class="empty-state">未选择任何环境线，请在上方勾选。</li>`;
    return;
  }
  ul.innerHTML = targets.map((t, i) => {
    const env = ENV_LINES.find(e => e.target === t);
    const name = env ? `${env.icon} ${esc(env.name)}` : `环境线 ${t}`;
    const res = state.checkResults[t] || {};
    const status = res.status || 'pending';
    const meta = CK_STATUS[status] || CK_STATUS.pending;
    const ms = res.ms != null ? (res.ms / 1000).toFixed(1) + 's' : (status === 'running' ? '…' : '—');
    const msg = res.message || (status === 'pending' ? '等待检测…' : '');
    return `<li class="check-item">
      <span class="ck-badge ${meta.cls}">${meta.label}</span>
      <span class="ck-name">${name}<small>BuildTarget ${t}${res.group ? ' · ' + esc(res.group) : ''}</small></span>
      <span class="ck-ms" title="耗时">${ms}</span>
      <span class="ck-msg" title="${esc(msg)}">${esc(msg)}</span>
      ${state.checkRunDir ? `<button class="btn btn-ghost btn-xs" data-log="${i}" title="打开本次检测结果目录">日志</button>` : ''}
    </li>`;
  }).join('');

  const cnt = $('checkResultCount');
  if (cnt) {
    const s = { ok: 0, fail: 0, running: 0, error: 0, timeout: 0, unsupported: 0 };
    for (const t of targets) {
      const st = (state.checkResults[t] || {}).status;
      if (st && s[st] != null) s[st]++;
    }
    cnt.textContent = `共 ${targets.length} 条环境线` +
      (s.ok ? ` · 通过 ${s.ok}` : '') +
      (s.fail ? ` · 失败 ${s.fail}` : '') +
      (s.error ? ` · 异常 ${s.error}` : '') +
      (s.timeout ? ` · 超时 ${s.timeout}` : '') +
      (s.running ? ` · 检测中 ${s.running}` : '') +
      (s.unsupported ? ` · 不支持 ${s.unsupported}` : '');
  }

  ul.querySelectorAll('button[data-log]').forEach(b => {
    b.onclick = () => { if (state.checkRunDir) openFolder(state.checkRunDir); };
  });
}

function checkLocalSummary() {
  const s = { ok: 0, fail: 0, error: 0, timeout: 0, unsupported: 0, cancelled: 0, total: 0 };
  for (const t of state.checkTargets) {
    const st = (state.checkResults[t] || {}).status;
    s.total++;
    if (st && s[st] != null) s[st]++;
  }
  return s;
}

function showCheckBanner(type, text) {
  const b = $('checkBanner');
  if (!b) return;
  b.className = `banner ${type}`;
  b.textContent = text;
  b.classList.remove('hidden');
}

function showCheckEnd(end) {
  const s = (end && end.summary) || checkLocalSummary();
  if ($('checkJobSummary') && state.checkRunDir) {
    $('checkJobSummary').textContent = `结果目录: ${state.checkRunDir}`;
  }
  $('btnCheckStart').disabled = false;
  $('btnCheckStop').disabled = true;
  if (end && end.ok) {
    showCheckBanner('ok', `✓ 环境编译检测完成：${s.ok} 条通过` + (s.unsupported ? `，${s.unsupported} 条不支持（跳过）` : ''));
    updateHeaderStatus('ok', '环境检测通过');
    showToast('环境编译检测完成：全部通过', 'ok');
  } else if (end && end.reason === 'cancelled') {
    showCheckBanner('fail', '⚠ 环境检测已被终止');
    updateHeaderStatus('idle', '检测已终止');
    showToast('环境检测已终止', 'info');
  } else {
    const parts = [];
    if (s.fail) parts.push(`失败 ${s.fail}`);
    if (s.error) parts.push(`异常 ${s.error}`);
    if (s.timeout) parts.push(`超时 ${s.timeout}`);
    showCheckBanner('fail', `✗ 环境编译检测存在问题：${parts.join('，') || '失败'}${s.unsupported ? `；${s.unsupported} 条不支持` : ''}`);
    updateHeaderStatus('fail', '检测存在失败项');
    showToast('环境检测完成，部分环境线未通过', 'err');
  }
  renderCheckResults();
}

async function doCheckStart() {
  const body = checkBody();
  if (!body.targets.length) {
    showToast('请至少选择一条环境线', 'err');
    return;
  }
  if (!body.projectPath) {
    showToast('请填写 Unity 项目路径', 'err');
    switchTab('profiles');
    return;
  }
  if (!body.engine.unityExe) {
    showToast('请填写 Unity.exe 路径（可在「引擎与高级配置」中自动探测）', 'err');
    switchTab('settings');
    return;
  }
  state.checkResults = {};
  state.checkRawLogs = '';
  state.checkRunDir = null;
  $('checkLogView').textContent = '';
  renderCheckResults();
  $('btnCheckStart').disabled = true;
  $('btnCheckStop').disabled = false;
  showCheckBanner('run', '正在启动环境编译检测…');
  updateHeaderStatus('running', '环境编译检测中…');
  try {
    const r = await api('/api/check/start', body);
    state.checkRunDir = r.runDir || null;
    showToast('环境编译检测已启动', 'ok');
  } catch (e) {
    showCheckBanner('fail', '启动失败：' + e.message);
    updateHeaderStatus('fail', '检测启动失败');
    $('btnCheckStart').disabled = false;
    $('btnCheckStop').disabled = true;
    showToast('启动失败: ' + e.message, 'err');
  }
}

async function doCheckStop() {
  try {
    await api('/api/check/stop');
    showToast('已请求终止环境检测', 'info');
  } catch (e) {
    console.warn(e);
  }
}

async function doCheckPreview() {
  const body = checkBody();
  if (!body.targets.length) {
    showToast('请至少选择一条环境线', 'err');
    return;
  }
  try {
    const r = await api('/api/check/preview', body);
    $('previewList').innerHTML = r.items.map(it => `<div class="cmd-block">
      <span class="tag">▶ ${esc(it.name)} · BuildTarget ${it.target}</span>
      <div class="hint">日志文件: ${esc(it.logFile)}</div>
      ${esc(it.command)}
    </div>`).join('') || '<div class="hint">（无预览命令）</div>';
    $('previewModal').classList.remove('hidden');
  } catch (e) {
    showToast('预览失败: ' + e.message, 'err');
  }
}

function appendCheckLog(text) {
  state.checkRawLogs += text;
  const lv = $('checkLogView');
  if (!lv) return;
  const search = $('checkLogSearch')?.value.trim();
  if (!search) lv.textContent = state.checkRawLogs.slice(-150000);
  else filterCheckLogView(search);
  if ($('checkAutoScroll')?.checked) lv.scrollTop = lv.scrollHeight;
}

function filterCheckLogView(search) {
  const lv = $('checkLogView');
  if (!lv) return;
  if (!search) {
    lv.textContent = state.checkRawLogs.slice(-150000);
    return;
  }
  lv.textContent = state.checkRawLogs.split('\n').filter(l => l.toLowerCase().includes(search.toLowerCase())).join('\n');
}

/* ─────────────── Markdown 渲染 ─────────────── */

function escapeHtmlMd(s) {
  return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function mdInline(s) {
  let t = escapeHtmlMd(s);
  t = t.replace(/`([^`]+)`/g, '<code>$1</code>');
  t = t.replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>');
  t = t.replace(/(^|[^*])\*([^*\n]+)\*/g, '$1<em>$2</em>');
  t = t.replace(/(^|[\w_])\_([^_\n]+)\_(?!\w)/g, '$1<em>$2</em>');
  t = t.replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>');
  return t;
}

function renderMarkdown(md) {
  if (!md) return '';
  const lines = md.split(/\r?\n/);
  const out = [];
  let inCode = false, codeLang = '', codeBuf = [];
  let inUl = false, inOl = false;

  function closeLists() {
    if (inUl) { out.push('</ul>'); inUl = false; }
    if (inOl) { out.push('</ol>'); inOl = false; }
  }

  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const fence = raw.match(/^```(\w*)/);
    if (fence) {
      if (inCode) {
        out.push(`<pre><code class="lang-${escapeHtmlMd(codeLang)}">${escapeHtmlMd(codeBuf.join('\n'))}</code></pre>`);
        codeBuf = []; inCode = false;
      } else {
        closeLists();
        inCode = true; codeLang = fence[1] || ''; codeBuf = [];
      }
      continue;
    }
    if (inCode) { codeBuf.push(raw); continue; }

    const hm = raw.match(/^(#{1,6})\s+(.*)$/);
    if (hm) {
      closeLists();
      const lv = hm[1].length;
      out.push(`<h${lv}>${mdInline(hm[2])}</h${lv}>`);
      continue;
    }

    const um = raw.match(/^[\*\-]\s+(.*)$/);
    if (um) {
      if (inOl) { out.push('</ol>'); inOl = false; }
      if (!inUl) { out.push('<ul>'); inUl = true; }
      out.push(`<li>${mdInline(um[1])}</li>`);
      continue;
    }

    const om = raw.match(/^\d+\.\s+(.*)$/);
    if (om) {
      if (inUl) { out.push('</ul>'); inUl = false; }
      if (!inOl) { out.push('<ol>'); inOl = true; }
      out.push(`<li>${mdInline(om[1])}</li>`);
      continue;
    }

    if (!raw.trim()) { closeLists(); continue; }
    closeLists();
    out.push(`<p>${mdInline(raw)}</p>`);
  }
  if (inCode) out.push(`<pre><code>${escapeHtmlMd(codeBuf.join('\n'))}</code></pre>`);
  closeLists();
  return out.join('\n');
}

/* ─────────────── AI 分析 ─────────────── */

async function doAiAnalyze() {
  if (!$('aiEnabled').checked) {
    showToast('请先在「引擎与高级配置」中开启 AI 诊断并填写 API Key', 'err');
    switchTab('settings');
    return;
  }
  $('aiModal').classList.remove('hidden');
  $('aiResult').innerHTML = '<div style="color: var(--accent-primary);">正在调取构建日志与 AI 接口分析中，请稍候…</div>';
  try {
    const r = await api('/api/ai/analyze', {});
    $('aiResult').innerHTML = renderMarkdown(r.analysis || '（AI 未返回分析内容）');
  } catch (e) {
    $('aiResult').innerHTML = `<div style="color: #fb7185;"><strong>分析失败：</strong>${esc(e.message)}</div>`;
  }
}

/* ─────────────── SSE 事件监听与进度渲染 ─────────────── */

let sse = null;
function initSSE() {
  if (sse) sse.close();
  sse = new EventSource('/api/build/events');

  sse.onmessage = e => {
    try {
      const d = JSON.parse(e.data);
      handleSSE(d);
    } catch { /* 忽略心跳 */ }
  };

  sse.onerror = () => {
    /* 自动重连 */
  };
}

function handleSSE(d) {
  // ── 环境编译检测事件 ──
  if (d.type === 'check-hello') {
    if (d.check) {
      state.checkJob = d.check;
      state.checkResults = d.check.results || {};
      state.checkRunDir = d.check.runDir || null;
      if (Array.isArray(d.check.targets) && d.check.targets.length) state.checkTargets = d.check.targets;
      renderCheckEnvChips();
      if (d.check.state === 'running' || d.check.state === 'starting') {
        $('btnCheckStart').disabled = true;
        $('btnCheckStop').disabled = false;
        showCheckBanner('run', `环境编译检测进行中（第 ${(d.check.index || 0) + 1}/${d.check.count} 条环境线）`);
        updateHeaderStatus('running', '环境编译检测中…');
        renderCheckResults();
      } else if (d.check.end) {
        showCheckEnd(d.check.end);
      } else {
        renderCheckResults();
      }
    }
    return;
  }

  if (d.type === 'check-start') {
    state.checkJob = { state: 'running', count: d.count, targets: d.targets };
    state.checkRunDir = d.runDir || null;
    if ($('checkJobSummary')) $('checkJobSummary').textContent = `结果目录: ${state.checkRunDir || '—'}`;
    state.checkResults = {};
    for (const t of d.targets) {
      state.checkResults[t] = { target: t, status: 'running', ms: null, message: '排队中…', ok: false };
    }
    renderCheckResults();
    showCheckBanner('run', `环境编译检测已启动，共 ${d.count} 条环境线`);
    updateHeaderStatus('running', '环境编译检测中…');
    $('btnCheckStart').disabled = true;
    $('btnCheckStop').disabled = false;
    return;
  }

  if (d.type === 'check-target-start') {
    state.checkResults[d.target] = { target: d.target, name: d.name, status: 'running', ms: null, message: '检测中…（启动 Unity 编译）', ok: false };
    renderCheckResults();
    return;
  }

  if (d.type === 'check-line') {
    appendCheckLog(d.text);
    return;
  }

  if (d.type === 'check-result') {
    state.checkResults[d.target] = d.result;
    renderCheckResults();
    return;
  }

  if (d.type === 'check-end') {
    showCheckEnd(d.end);
    return;
  }

  // ── 版本管理事件 ──
  if (d.type === 'vcs-hello') {
    if (d.vcs) {
      state.vcsJob = d.vcs;
      for (const id of Object.keys(d.vcs.results || {})) {
        const res = d.vcs.results[id];
        const st = state.vcsNodeStates[id] || (state.vcsNodeStates[id] = {});
        st.status = res.status || (res.ok ? 'ok' : 'fail');
        st.type = res.type || st.type;
        st.message = res.message || '';
      }
      if (d.vcs.state === 'running' || d.vcs.state === 'starting') {
        $('btnVcsStop').disabled = false;
        $('btnVcsUpdateAll').disabled = true;
        showVcsBanner('run', `版本管理更新进行中（第 ${(d.vcs.index || 0) + 1}/${d.vcs.count} 个节点）`);
        updateHeaderStatus('running', '版本更新中…');
        renderVcs();
      } else if (d.vcs.end) {
        showVcsEnd(d.vcs.end);
      }
    }
    return;
  }

  if (d.type === 'vcs-start') {
    state.vcsJob = { state: 'running', count: d.count, index: 0 };
    showVcsBanner('run', `版本管理更新已启动，共 ${d.count} 个节点（更新前自动还原未提交更改）`);
    updateHeaderStatus('running', '版本更新中…');
    $('btnVcsStop').disabled = false;
    $('btnVcsUpdateAll').disabled = true;
    return;
  }

  if (d.type === 'vcs-node-start') {
    if (state.vcsJob) state.vcsJob.index = d.index;
    const st = state.vcsNodeStates[d.nodeId] || (state.vcsNodeStates[d.nodeId] = {});
    st.status = 'updating';
    st.message = d.path ? '还原并更新中…' : '路径为空，跳过';
    renderVcs();
    appendVcsLog(`\n▶ 开始更新节点：${d.name}（${d.path || '路径为空'}）\n`);
    return;
  }

  if (d.type === 'vcs-line') {
    appendVcsLog(d.text);
    return;
  }

  if (d.type === 'vcs-node-end') {
    const st = state.vcsNodeStates[d.nodeId] || (state.vcsNodeStates[d.nodeId] = {});
    st.status = ['ok', 'fail', 'skipped', 'cancelled'].includes(d.status) ? d.status : (d.ok ? 'ok' : 'fail');
    st.type = d.type || st.type;
    st.message = d.message || '';
    renderVcs();
    return;
  }

  if (d.type === 'vcs-end') {
    showVcsEnd(d.end || d);
    return;
  }

  if (d.type === 'hello') {
    if (d.job) {
      state.job = d.job;
      state.jobProgress = d.job.progress || {};
      if (d.job.state === 'running' || d.job.state === 'starting') {
        $('progressArea').classList.remove('hidden');
        buildProgressFromJob(d.job);
        showBanner('run', `正在批量打包（第 ${d.job.index + 1}/${d.job.count} 项）`);
        updateHeaderStatus('running', '批量打包中…');
        $('btnStart').disabled = true;
        $('btnStop').disabled = false;
      } else if (d.job.end) {
        showJobEnd(d.job.end);
      }
    }
    return;
  }

  if (d.type === 'job-start') {
    state.job = { state: 'running', count: d.count, index: 0 };
    state.jobProgress = {};
    $('progressArea').classList.remove('hidden');
    buildProgressInit();
    showBanner('run', `批量打包已启动，共 ${d.count} 个 Profile`);
    updateHeaderStatus('running', '批量打包中…');
    $('btnStart').disabled = true;
    $('btnStop').disabled = false;
    $('btnAi').disabled = true;
    return;
  }

  if (d.type === 'profile-start') {
    if (state.job) state.job.index = d.index;
    const tag = `[${d.index + 1}/${state.job ? state.job.count : '?'}]`;
    const devTag = d.dev ? ' [DEV]' : '';
    appendLog(`\n====================================================\n▶ ${tag} 开始构建: ${d.name} (${d.target})${devTag}\n  输出: ${d.output}\n  命令: ${d.cmd}\n====================================================\n`);
    updateProgressItem(d.index, 'running', '启动中…', null);
    showBanner('run', `正在构建第 ${d.index + 1}/${state.job ? state.job.count : '?'} 项：${d.name}`);
    return;
  }

  if (d.type === 'progress') {
    updateProgressItem(d.index, 'running', d.stage, d.pct);
    updateGlobalProgressBar(d.index, d.stage, d.pct);
    return;
  }

  if (d.type === 'line') {
    appendLog(d.text);
    return;
  }

  if (d.type === 'notice') {
    appendLog('\n' + d.text + '\n');
    return;
  }

  if (d.type === 'output') {
    state.outputs.unshift(d);
    renderOutputs();
    return;
  }

  if (d.type === 'profile-end') {
    updateProgressItem(d.index, d.ok ? 'done' : 'fail', d.ok ? '构建成功' : `失败(退出码 ${d.exitCode})`, d.ok ? 100 : null);
    return;
  }

  if (d.type === 'job-end' || d.type === 'done') {
    const end = d.end || d;
    showJobEnd(end);
    return;
  }
}

function updateHeaderStatus(stateCls, text) {
  const badge = $('liveStatusBadge');
  const txt = $('headerStatusText');
  if (badge) badge.className = `status-pill ${stateCls}`;
  if (txt) txt.textContent = text;
}

function showBanner(type, text) {
  const b = $('jobBanner');
  if (!b) return;
  b.className = `banner ${type}`;
  b.textContent = text;
  b.classList.remove('hidden');
}

function hideBanner() {
  $('jobBanner')?.classList.add('hidden');
}

function showJobEnd(end) {
  $('btnStart').disabled = false;
  $('btnStop').disabled = true;
  const pbar = $('progressBarWrap');
  const pbarFill = $('pbarFill');

  if (end.ok) {
    showBanner('ok', '✓ 批量打包全部完成！产物已归档。');
    updateHeaderStatus('ok', '全部构建成功');
    showToast('批量打包全部完成！', 'ok');
    if (pbarFill) { pbarFill.style.width = '100%'; pbarFill.className = 'progress-bar-fill ok'; }
    $('btnAi').disabled = true;
  } else if (end.reason === 'cancelled') {
    showBanner('fail', '⚠ 构建已被用户终止');
    updateHeaderStatus('idle', '已终止');
    showToast('构建已终止', 'err');
    $('btnAi').disabled = false;
  } else {
    showBanner('fail', `✗ 构建失败 (${end.reason || 'error'})`);
    updateHeaderStatus('fail', '构建失败');
    showToast('构建失败，可点击「AI 失败分析」排障', 'err');
    if (pbarFill) pbarFill.className = 'progress-bar-fill fail';
    $('btnAi').disabled = false;
  }
  refreshOutputs();
}

function updateGlobalProgressBar(idx, stage, pct) {
  const wrap = $('progressBarWrap');
  if (!wrap) return;
  wrap.classList.remove('hidden');
  const fill = $('pbarFill');
  const st = $('pbarStage');
  const pt = $('pbarPct');

  const total = state.queue.length || 1;
  const basePct = (idx / total) * 100;
  const itemPct = pct != null ? (pct / total) : 0;
  const overall = Math.min(100, Math.round(basePct + itemPct));

  if (fill) {
    fill.style.width = overall + '%';
    fill.className = 'progress-bar-fill';
  }
  if (st) st.textContent = `[${idx + 1}/${total}] ${stage || '进行中…'}`;
  if (pt) pt.textContent = overall + '%';
}

function buildProgressInit() {
  const list = $('progressList');
  if (!list) return;
  list.innerHTML = state.queue.map((p, i) =>
    `<div class="prog-item" id="prog-${i}">
      <span class="st pending">待执行</span>
      <span class="pm">
        ${esc(p.name)} · ${esc(p.targetName)}
        <small class="prog-detail">排队中…</small>
      </span>
    </div>`
  ).join('');
}

function buildProgressFromJob(j) {
  const list = $('progressList');
  if (!list || !j.queue) return;
  list.innerHTML = j.queue.map((p, i) => {
    const isDone = i < j.index;
    const isCur = i === j.index;
    const stCls = isDone ? 'done' : isCur ? 'running' : 'pending';
    const stTxt = isDone ? '完成' : isCur ? '构建中' : '待执行';
    return `<div class="prog-item" id="prog-${i}">
      <span class="st ${stCls}">${stTxt}</span>
      <span class="pm">
        ${esc(p.name)} · ${esc(p.targetName)}
        <small class="prog-detail">${isCur ? '进行中…' : ''}</small>
      </span>
    </div>`;
  }).join('');
}

function updateProgressItem(idx, st, stage, pct) {
  const el = document.getElementById('prog-' + idx);
  if (!el) return;
  const stEl = el.querySelector('.st');
  const dtEl = el.querySelector('.prog-detail');
  if (stEl) {
    stEl.className = 'st ' + st;
    stEl.textContent = st === 'running' ? '构建中' : st === 'done' ? '完成' : st === 'fail' ? '失败' : '待执行';
  }
  if (dtEl) {
    const pctTxt = pct != null ? ` (${pct}%)` : '';
    dtEl.textContent = (stage || '') + pctTxt;
  }
}

/* ─────────────── 日志视图与搜索过滤 ─────────────── */

function appendLog(text) {
  state.rawLogs += text;
  const lv = $('logView');
  if (!lv) return;

  const search = $('logSearch')?.value.trim();
  if (!search) {
    lv.textContent = state.rawLogs.slice(-150000);
  } else {
    filterLogView(search);
  }

  if ($('autoScroll')?.checked) {
    lv.scrollTop = lv.scrollHeight;
  }
}

function filterLogView(search) {
  const lv = $('logView');
  if (!lv) return;
  if (!search) {
    lv.textContent = state.rawLogs.slice(-150000);
    return;
  }
  const lines = state.rawLogs.split('\n');
  const filtered = lines.filter(l => l.toLowerCase().includes(search.toLowerCase()));
  lv.textContent = filtered.join('\n');
}

/* ─────────────── 版本管理（SVN / Git 节点与分组） ─────────────── */

const VCS_STATUS = {
  idle:      { label: '空闲',   cls: 'pending' },
  probing:   { label: '探测中', cls: 'running' },
  updating:  { label: '更新中', cls: 'running' },
  ok:        { label: '成功',   cls: 'ok' },
  fail:      { label: '失败',   cls: 'fail' },
  skipped:   { label: '跳过',   cls: 'unsupported' },
  cancelled: { label: '已取消', cls: 'cancelled' },
};

function vcsNodeById(id) { return state.vcsNodes.find(n => String(n.id) === String(id)); }
function vcsGroupById(id) { return state.vcsGroups.find(g => String(g.id) === String(id)); }
function vcsGroupOf(nodeId) {
  return state.vcsGroups.find(g => (g.nodeIds || []).some(x => String(x) === String(nodeId))) || null;
}
function vcsUngrouped() {
  return state.vcsNodes.filter(n => !vcsGroupOf(n.id));
}
function vcsBusy() {
  return !!(state.vcsJob && (state.vcsJob.state === 'running' || state.vcsJob.state === 'starting'));
}

/* 保存节点 / 分组配置（防抖） */
let vcsSaveTimer = null;
function scheduleVcsSave() {
  clearTimeout(vcsSaveTimer);
  vcsSaveTimer = setTimeout(async () => {
    try {
      await api('/api/vcs/save', { nodes: state.vcsNodes, groups: state.vcsGroups });
    } catch (e) {
      console.warn('vcs save:', e.message);
    }
  }, 400);
}

function vcsNodeCard(n) {
  const st = state.vcsNodeStates[n.id] || {};
  const status = st.status || 'idle';
  const meta = VCS_STATUS[status] || VCS_STATUS.idle;
  const type = st.type || (n.type !== 'auto' ? n.type : 'none');
  const typeBadge = type === 'git'
    ? '<span class="badge vcs-badge-git">Git</span>'
    : type === 'svn'
      ? '<span class="badge vcs-badge-svn">SVN</span>'
      : '<span class="badge vcs-badge-none">未检测</span>';

  const sub = [];
  if (st.branch) sub.push(`分支 ${esc(st.branch)}`);
  if (st.revision) sub.push(type === 'svn' ? 'r' + esc(st.revision) : '#' + esc(st.revision));
  if (st.dirty) sub.push(`<span class="vcs-dirty" title="存在未提交更改，更新前会自动还原">⚠ ${st.dirtyCount || ''} 处未提交</span>`);
  if (st.message) sub.push(esc(st.message));

  const pathTxt = n.path
    ? esc(n.path)
    : '<span class="vcs-path-empty">（路径为空，不执行更新）</span>';

  return `<div class="vcs-node ${status === 'updating' || status === 'probing' ? 'busy' : ''}" draggable="true" data-nid="${esc(n.id)}" title="拖动 ⠿ 手柄将节点归入分组">
    <span class="vcs-drag" title="拖动归组">⠿</span>
    <span class="vcs-st ${meta.cls}">${meta.label}</span>
    ${typeBadge}
    <span class="vcs-node-main">
      <span class="vcs-node-name">${esc(n.name)}</span>
      <small class="vcs-node-path">${pathTxt}</small>
      ${sub.length ? `<small class="vcs-node-meta">${sub.join(' · ')}</small>` : ''}
    </span>
    <span class="vcs-node-actions">
      ${vcsBusy() ? '' : `<button class="btn btn-xs btn-ghost" data-vcs-update="${esc(n.id)}" title="还原未提交更改并更新">更新</button>`}
      <button class="btn btn-xs btn-ghost" data-vcs-probe="${esc(n.id)}" title="重新检测类型与状态">刷新</button>
      <button class="btn btn-xs btn-ghost" data-vcs-edit="${esc(n.id)}" title="编辑节点">编辑</button>
      <button class="btn btn-xs btn-ghost" data-vcs-del="${esc(n.id)}" title="删除节点">×</button>
    </span>
  </div>`;
}

function vcsGroupCard(g) {
  const nodes = (g.nodeIds || []).map(vcsNodeById).filter(Boolean);
  return `<div class="card glass-card vcs-group-card" data-gid="${esc(g.id)}">
    <div class="card-header">
      <div class="card-title" style="flex: 1; min-width: 0;">
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path></svg>
        <input type="text" class="vcs-group-name" value="${esc(g.name)}" data-gname="${esc(g.id)}" title="点击重命名分组（回车 / 失焦保存）">
        <span class="counter-badge">${nodes.length} 个节点</span>
      </div>
      <div class="card-actions">
        <button class="btn btn-sm btn-primary" data-vcs-update-group="${esc(g.id)}" ${vcsBusy() ? 'disabled' : ''}>更新分组</button>
        <button class="btn btn-sm btn-ghost" data-vcs-del-group="${esc(g.id)}" title="删除分组（节点保留，回到未分组）">删除</button>
      </div>
    </div>
    <div class="card-body vcs-group-body">
      ${nodes.map(n => vcsNodeCard(n)).join('') || '<div class="empty-state vcs-drop-hint">空分组 · 将节点卡片拖入此处归组</div>'}
    </div>
  </div>`;
}

function bindVcsEvents() {
  document.querySelectorAll('[data-vcs-update]').forEach(b => {
    b.onclick = () => doVcsUpdate([b.dataset.vcsUpdate], '更新节点');
  });
  document.querySelectorAll('[data-vcs-update-group]').forEach(b => {
    b.onclick = () => {
      const g = vcsGroupById(b.dataset.vcsUpdateGroup);
      if (!g) return;
      doVcsUpdate((g.nodeIds || []).slice(), '更新分组「' + (g.name || '') + '」');
    };
  });
  document.querySelectorAll('[data-vcs-probe]').forEach(b => {
    b.onclick = () => doVcsProbe([b.dataset.vcsProbe]);
  });
  document.querySelectorAll('[data-vcs-edit]').forEach(b => {
    b.onclick = () => openVcsNodeModal(b.dataset.vcsEdit);
  });
  document.querySelectorAll('[data-vcs-del]').forEach(b => {
    b.onclick = () => deleteVcsNode(b.dataset.vcsDel);
  });
  document.querySelectorAll('[data-vcs-del-group]').forEach(b => {
    b.onclick = () => deleteVcsGroup(b.dataset.vcsDelGroup);
  });
  document.querySelectorAll('.vcs-group-name').forEach(inp => {
    inp.onchange = () => renameVcsGroup(inp.dataset.gname, inp.value);
    inp.onkeydown = e => { if (e.key === 'Enter') inp.blur(); };
  });
}

function clearVcsDragOver() {
  document.querySelectorAll('.vcs-group-card, #vcsUngroupedCard').forEach(el => el.classList.remove('drag-over'));
}

function bindVcsDnD() {
  document.querySelectorAll('.vcs-node[draggable="true"]').forEach(el => {
    el.addEventListener('dragstart', e => {
      if (e.target.closest('button, input, select, textarea')) { e.preventDefault(); return; }
      const id = el.dataset.nid;
      state.vcsDragId = id;
      e.dataTransfer.setData('text/plain', id);
      e.dataTransfer.effectAllowed = 'move';
      el.classList.add('dragging');
    });
    el.addEventListener('dragend', () => {
      state.vcsDragId = null;
      el.classList.remove('dragging');
      clearVcsDragOver();
    });
  });

  const bindDrop = (el, groupId) => {
    el.addEventListener('dragover', e => {
      if (!state.vcsDragId) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = 'move';
      el.classList.add('drag-over');
    });
    el.addEventListener('dragleave', () => el.classList.remove('drag-over'));
    el.addEventListener('drop', e => {
      e.preventDefault();
      el.classList.remove('drag-over');
      const id = state.vcsDragId || e.dataTransfer.getData('text/plain');
      if (id) moveVcsNodeToGroup(id, groupId);
    });
  };

  document.querySelectorAll('.vcs-group-card').forEach(card => bindDrop(card, card.dataset.gid));
  const un = $('vcsUngroupedCard');
  if (un) bindDrop(un, null);
}

function moveVcsNodeToGroup(nodeId, groupId) {
  if (!vcsNodeById(nodeId)) return;
  // 先从所有分组中移除
  state.vcsGroups.forEach(g => {
    g.nodeIds = (g.nodeIds || []).filter(x => String(x) !== String(nodeId));
  });
  if (groupId) {
    const g = vcsGroupById(groupId);
    if (g) {
      g.nodeIds = g.nodeIds || [];
      if (!g.nodeIds.some(x => String(x) === String(nodeId))) g.nodeIds.push(nodeId);
    }
  }
  renderVcs();
  scheduleVcsSave();
  showToast(groupId ? '节点已归入分组' : '节点已移出分组', 'ok');
}

function addVcsGroup() {
  state.vcsGroups.push({
    id: 'g' + Date.now().toString(36) + Math.random().toString(36).slice(2, 6),
    name: '新分组',
    nodeIds: [],
  });
  renderVcs();
  scheduleVcsSave();
  showToast('已新建分组', 'ok');
}

function renameVcsGroup(id, name) {
  const g = vcsGroupById(id);
  if (!g) return;
  g.name = (name || '').trim() || '新分组';
  scheduleVcsSave();
}

function deleteVcsGroup(id) {
  if (!confirm('确定删除分组「' + (vcsGroupById(id)?.name || id) + '」？分组内节点将回到未分组列表。')) return;
  state.vcsGroups = state.vcsGroups.filter(g => String(g.id) !== String(id));
  renderVcs();
  scheduleVcsSave();
  showToast('已删除分组（节点保留）', 'info');
}

function deleteVcsNode(id) {
  const n = vcsNodeById(id);
  if (!n) return;
  if (!confirm('确定删除节点「' + (n.name || id) + '」？')) return;
  state.vcsNodes = state.vcsNodes.filter(x => String(x.id) !== String(id));
  state.vcsGroups.forEach(g => {
    g.nodeIds = (g.nodeIds || []).filter(x => String(x) !== String(id));
  });
  delete state.vcsNodeStates[id];
  renderVcs();
  scheduleVcsSave();
  showToast('节点已删除', 'info');
}

function renderVcs() {
  const un = vcsUngrouped();
  const unArea = $('vcsUngroupedArea');
  if (unArea) {
    unArea.innerHTML = un.map(n => vcsNodeCard(n)).join('') ||
      '<div class="empty-state vcs-drop-hint">暂无节点 · 点击「添加节点」创建第一条仓库路径（默认路径为空，不执行更新）</div>';
  }
  const uc = $('vcsUngroupedCount');
  if (uc) uc.textContent = un.length;

  const ga = $('vcsGroupsArea');
  if (ga) ga.innerHTML = state.vcsGroups.length ? state.vcsGroups.map(vcsGroupCard).join('') : '';
  const eh = $('vcsEmptyHint');
  if (eh) eh.classList.toggle('hidden', state.vcsGroups.length > 0);

  bindVcsEvents();
  bindVcsDnD();
}

/* 探测：自动检测路径类型（Git / SVN）+ 分支 / 版本 / 未提交状态 */
async function doVcsProbe(ids) {
  const targets = (ids || state.vcsNodes.map(n => n.id)).filter(id => vcsNodeById(id));
  if (!targets.length) return;
  for (const id of targets) {
    const n = vcsNodeById(id);
    const st = state.vcsNodeStates[id] || (state.vcsNodeStates[id] = {});
    if (!n.path.trim()) {
      st.status = 'idle';
      st.type = 'none';
      st.branch = ''; st.revision = ''; st.dirty = false; st.dirtyCount = 0;
      st.message = '路径为空';
      continue;
    }
    st.status = 'probing';
    st.message = '';
  }
  renderVcs();
  try {
    const r = await api('/api/vcs/probe', { ids: targets });
    for (const res of r.results || []) {
      const st = state.vcsNodeStates[res.id] || (state.vcsNodeStates[res.id] = {});
      st.type = res.type || 'none';
      st.branch = res.branch || '';
      st.revision = res.revision || '';
      st.dirty = !!res.dirty;
      st.dirtyCount = res.dirtyCount || 0;
      st.message = res.error || '';
      st.status = (res.type === 'none' || res.ok) ? 'idle' : 'fail';
    }
    renderVcs();
  } catch (e) {
    showToast('状态探测失败: ' + e.message, 'err');
    for (const id of targets) {
      const st = state.vcsNodeStates[id] || {};
      if (st.status === 'probing') st.status = 'idle';
    }
    renderVcs();
  }
}

/* 更新：还原未提交更改 → 拉取 / 更新（路径为空节点直接跳过） */
async function doVcsUpdate(ids, label) {
  const valid = (ids || []).filter(id => vcsNodeById(id));
  if (!valid.length) {
    showToast(label + '：没有可更新的节点', 'err');
    return;
  }
  if (!valid.some(id => vcsNodeById(id).path.trim())) {
    showToast('所选节点路径均为空，未执行任何更新操作', 'err');
    return;
  }
  $('btnVcsStop').disabled = false;
  $('btnVcsUpdateAll').disabled = true;
  showVcsBanner('run', '正在启动版本管理更新…');
  updateHeaderStatus('running', '版本更新中…');
  for (const id of valid) {
    const st = state.vcsNodeStates[id] || (state.vcsNodeStates[id] = {});
    st.status = 'updating';
    st.message = '排队中…';
  }
  renderVcs();
  try {
    await api('/api/vcs/update', { ids: valid });
    showToast('版本更新已启动', 'ok');
  } catch (e) {
    showVcsBanner('fail', '启动失败：' + e.message);
    updateHeaderStatus('fail', '更新启动失败');
    $('btnVcsStop').disabled = true;
    $('btnVcsUpdateAll').disabled = false;
    for (const id of valid) {
      const st = state.vcsNodeStates[id] || {};
      if (st.status === 'updating') st.status = 'idle';
    }
    renderVcs();
    showToast('启动失败: ' + e.message, 'err');
  }
}

async function doVcsStop() {
  try {
    await api('/api/vcs/stop');
    showToast('已请求终止版本更新', 'info');
  } catch (e) {
    console.warn(e);
  }
}

function showVcsBanner(type, text) {
  const b = $('vcsBanner');
  if (!b) return;
  b.className = `banner ${type}`;
  b.textContent = text;
  b.classList.remove('hidden');
}

function showVcsEnd(end) {
  $('btnVcsStop').disabled = true;
  $('btnVcsUpdateAll').disabled = false;
  const s = (end && end.summary) || {};
  if (end && end.ok) {
    showVcsBanner('ok', `✓ 版本管理更新完成：成功 ${s.ok || 0}${s.skipped ? '，跳过 ' + s.skipped : ''}`);
    updateHeaderStatus('ok', '版本更新完成');
    showToast('版本管理更新完成', 'ok');
  } else if (end && end.reason === 'cancelled') {
    showVcsBanner('fail', '⚠ 版本更新已终止');
    updateHeaderStatus('idle', '版本更新已终止');
    showToast('版本更新已终止', 'info');
  } else {
    showVcsBanner('fail', `✗ 版本更新结束：成功 ${s.ok || 0}，失败 ${s.fail || 0}${s.skipped ? '，跳过 ' + s.skipped : ''}`);
    updateHeaderStatus('fail', '版本更新存在失败项');
    showToast('版本更新完成，部分节点失败', 'err');
  }
  // 自动刷新已更新节点的最新状态（分支 / 版本）
  const updated = Object.keys(state.vcsNodeStates).filter(id =>
    ['ok', 'fail', 'skipped', 'cancelled'].includes((state.vcsNodeStates[id] || {}).status));
  if (updated.length) doVcsProbe(updated);
}

/* 节点添加 / 编辑弹窗 */
function openVcsNodeModal(id) {
  state.vcsEditId = id || null;
  const n = id ? vcsNodeById(id) : null;
  $('vcsNodeModalTitle').textContent = n ? `编辑节点：${n.name || ''}` : '添加版本管理节点';
  $('vcsNodeName').value = n ? (n.name || '') : '';
  $('vcsNodePath').value = n ? (n.path || '') : '';
  $('vcsNodeType').value = (n && (n.type === 'git' || n.type === 'svn')) ? n.type : 'auto';
  $('vcsNodeProbeResult').textContent = '';
  $('vcsNodeModal').classList.remove('hidden');
  $('vcsNodeName').focus();
  $('vcsNodeName').select();
}

function closeVcsNodeModal() {
  $('vcsNodeModal').classList.add('hidden');
  state.vcsEditId = null;
}

function saveVcsNode() {
  const name = $('vcsNodeName').value.trim();
  const path = $('vcsNodePath').value.trim();
  const type = $('vcsNodeType').value || 'auto';
  if (!name) {
    showToast('请填写节点名称', 'err');
    $('vcsNodeName').focus();
    return;
  }
  if (state.vcsEditId) {
    const n = vcsNodeById(state.vcsEditId);
    if (n) {
      n.name = name;
      n.path = path;
      n.type = type;
    }
    const st = state.vcsNodeStates[state.vcsEditId] || (state.vcsNodeStates[state.vcsEditId] = {});
    st.status = 'idle'; st.type = ''; st.message = ''; st.dirty = false;
    doVcsProbe([state.vcsEditId]);
  } else {
    const id = 'n' + Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
    state.vcsNodes.push({ id, name, path, type });
    state.vcsNodeStates[id] = { status: 'idle', type: '', message: '' };
    doVcsProbe([id]);
  }
  closeVcsNodeModal();
  renderVcs();
  scheduleVcsSave();
  showToast('节点已保存', 'ok');
}

async function probeVcsNodeInModal() {
  const path = $('vcsNodePath').value.trim();
  const type = $('vcsNodeType').value || 'auto';
  const box = $('vcsNodeProbeResult');
  if (!path) {
    box.textContent = '路径为空：填路径后即可检测类型。';
    box.className = 'vcs-probe-result';
    return;
  }
  box.textContent = '探测中…';
  box.className = 'vcs-probe-result';
  try {
    const r = await api('/api/vcs/probe', { nodes: [{ id: 'adhoc', name: 'adhoc', path, type }] });
    const res = (r.results || [])[0];
    if (!res) throw new Error('无探测结果');
    if (res.type === 'git' || res.type === 'svn') {
      const extra = [];
      if (res.branch) extra.push('分支 ' + res.branch);
      if (res.revision) extra.push(res.type === 'svn' ? 'r' + res.revision : '#' + res.revision);
      if (res.dirty) extra.push(`${res.dirtyCount || ''} 处未提交`);
      box.textContent = `✓ 检测到 ${res.type.toUpperCase()}：${extra.join(' · ')}`;
      box.className = 'vcs-probe-result ok';
    } else {
      box.textContent = '✗ ' + (res.error || '未检测到 Git / SVN 工作副本');
      box.className = 'vcs-probe-result err';
    }
  } catch (e) {
    box.textContent = '✗ 探测失败：' + e.message;
    box.className = 'vcs-probe-result err';
  }
}

/* 版本管理日志视图 */
function appendVcsLog(text) {
  state.vcsRawLogs += text;
  if (state.vcsRawLogs.length > 300000) state.vcsRawLogs = state.vcsRawLogs.slice(-300000);
  const lv = $('vcsLogView');
  if (!lv) return;
  const search = $('vcsLogSearch')?.value.trim();
  if (!search) lv.textContent = state.vcsRawLogs;
  else filterVcsLogView(search);
  if ($('vcsAutoScroll')?.checked) lv.scrollTop = lv.scrollHeight;
}

function filterVcsLogView(search) {
  const lv = $('vcsLogView');
  if (!lv) return;
  if (!search) {
    lv.textContent = state.vcsRawLogs;
    return;
  }
  lv.textContent = state.vcsRawLogs.split('\n').filter(l => l.toLowerCase().includes(search.toLowerCase())).join('\n');
}

/* ─────────────── 页面初始化与事件绑定 ─────────────── */

async function init() {
  // 1. 侧边导航切换
  document.querySelectorAll('.sidebar-nav .nav-item').forEach(b => {
    b.onclick = () => switchTab(b.dataset.tab);
  });

  // 2. 占位符插入按键
  document.querySelectorAll('[data-insert]').forEach(b => {
    b.onclick = () => {
      const ta = $('template');
      ta.value += (ta.value ? ' ' : '') + b.dataset.insert;
      scheduleSave();
    };
  });

  // 全局命名占位符插入
  document.querySelectorAll('[data-insert-name]').forEach(b => {
    b.onclick = () => {
      const input = $('artifactNameTemplate');
      input.value += b.dataset.insertName;
      scheduleSave();
    };
  });

  // 弹窗命名占位符插入
  document.querySelectorAll('[data-insert-pcname]').forEach(b => {
    b.onclick = () => {
      const input = $('pcNameTemplate');
      input.value += b.dataset.insertPcname;
    };
  });

  // 3. 密码框眼睛切换
  document.querySelectorAll('.btn-toggle-eye').forEach(btn => {
    btn.onclick = () => {
      const targetInput = $(btn.dataset.target);
      if (!targetInput) return;
      if (targetInput.type === 'password') {
        targetInput.type = 'text';
        btn.textContent = '🔒';
      } else {
        targetInput.type = 'password';
        btn.textContent = '👁';
      }
    };
  });

  // 4. 按钮事件
  $('btnUseRepo').onclick = () => { $('projectPath').value = '..\\..'; scheduleSave(); doScan(); };
  $('btnScan').onclick = doScan;
  $('btnDetect').onclick = doDetect;
  $('btnCliProbe').onclick = doCliProbe;
  $('btnCliOpen').onclick = doCliOpen;
  $('btnPreview').onclick = doPreview;
  $('btnStart').onclick = doStart;
  $('btnStop').onclick = doStop;
  $('btnAi').onclick = doAiAnalyze;
  $('btnOpenOutBase').onclick = openOutputBaseFolder;
  $('btnOpenArtifactsDir').onclick = openOutputBaseFolder;
  $('btnRefreshOutputs').onclick = refreshOutputs;

  // 环境编译检测按钮
  $('btnCheckStart').onclick = doCheckStart;
  $('btnCheckStop').onclick = doCheckStop;
  $('btnCheckPreview').onclick = doCheckPreview;
  $('btnOpenCheckDir').onclick = () => {
    const p = state.checkRunDir || checkBody().projectPath;
    openFolder(p || 'Builds');
  };
  $('btnCheckUseRepo').onclick = () => { $('checkProjectPath').value = '..\\..'; scheduleSave(); };
  $('btnClearCheckResults').onclick = () => { state.checkResults = {}; renderCheckResults(); };
  $('btnClearCheckLog').onclick = () => { state.checkRawLogs = ''; $('checkLogView').textContent = '— 环境检测日志已清空 —'; };
  $('btnCopyCheckLog').onclick = () => copyToClipboard(state.checkRawLogs, '已复制环境检测日志');
  $('btnExpandCheckLog').onclick = () => $('checkTerminalBox').classList.toggle('fullscreen');
  $('checkLogSearch').oninput = e => filterCheckLogView(e.target.value.trim());
  renderCheckEnvChips();

  // 版本管理按钮
  $('btnVcsAddNode').onclick = () => openVcsNodeModal(null);
  $('btnVcsAddGroup').onclick = addVcsGroup;
  $('btnVcsUpdateAll').onclick = () => doVcsUpdate(state.vcsNodes.map(n => n.id), '更新全部节点');
  $('btnVcsProbe').onclick = () => doVcsProbe(state.vcsNodes.map(n => n.id));
  $('btnVcsStop').onclick = doVcsStop;
  $('btnVcsNodeSave').onclick = saveVcsNode;
  $('btnVcsNodeCancel').onclick = closeVcsNodeModal;
  $('btnVcsNodeCloseTop').onclick = closeVcsNodeModal;
  $('btnVcsNodeProbe').onclick = probeVcsNodeInModal;
  $('btnClearVcsLog').onclick = () => { state.vcsRawLogs = ''; $('vcsLogView').textContent = '— 版本管理日志已清空 —'; };
  $('btnCopyVcsLog').onclick = () => copyToClipboard(state.vcsRawLogs, '已复制版本管理日志');
  $('btnExpandVcsLog').onclick = () => $('vcsTerminalBox').classList.toggle('fullscreen');
  $('vcsLogSearch').oninput = e => filterVcsLogView(e.target.value.trim());

  // 5. 弹窗控制
  $('btnClosePreview').onclick = () => $('previewModal').classList.add('hidden');
  $('btnClosePreviewTop').onclick = () => $('previewModal').classList.add('hidden');
  $('btnCopyPreview').onclick = () => {
    const text = Array.from(document.querySelectorAll('#previewList .cmd-block')).map(el => el.textContent).join('\n\n');
    copyToClipboard(text, '已复制���部预览命令');
  };

  $('pcSave').onclick = saveProfileCfg;
  $('pcCancel').onclick = cancelProfileCfg;
  $('btnCancelProfileCfgTop').onclick = cancelProfileCfg;

  $('aiClose').onclick = () => $('aiModal').classList.add('hidden');
  $('btnAiCloseTop').onclick = () => $('aiModal').classList.add('hidden');
  $('btnCopyAi').onclick = () => copyToClipboard($('aiResult').innerText, '已复制 AI 分析结果');

  // 6. 终端日志操作
  $('btnClearLog').onclick = () => {
    state.rawLogs = '';
    $('logView').textContent = '— 日志已清空 —';
    showToast('日志已清空', 'info');
  };
  $('btnCopyLog').onclick = () => copyToClipboard(state.rawLogs, '已复制全部日志');
  $('btnExpandLog').onclick = () => $('terminalBox').classList.toggle('fullscreen');
  $('logSearch').oninput = e => filterLogView(e.target.value.trim());

  // 7. Profile 平台筛选 Chips
  $('filterChips').querySelectorAll('.chip').forEach(b => {
    b.onclick = () => {
      $('filterChips').querySelectorAll('.chip').forEach(x => x.classList.remove('active'));
      b.classList.add('active');
      state.filter = b.dataset.f;
      renderProfiles();
    };
  });

  // 8. 引擎模式单选
  document.querySelectorAll('input[name="engineMode"]').forEach(r => {
    r.onchange = () => {
      toggleEnginePanels();
      scheduleSave();
    };
  });

  // 9. 表单输入自动暂存
  ['projectPath', 'profileDir', 'unityExe', 'cliPath', 'template', 'artifactNameTemplate', 'outputBase', 'successDir', 'buildNumber', 'proxyHost', 'proxyPort', 'aiBaseUrl', 'aiModel', 'aiApiKey', 'checkProjectPath', 'checkUnityExe', 'checkTimeout'].forEach(id => {
    const el = $(id);
    if (el) el.oninput = scheduleSave;
  });
  ['nographics', 'defaultAutoZip', 'defaultDevBuild', 'defaultBuildAddressables', 'stopOnError', 'proxyEnabled', 'aiEnabled', 'checkNographics'].forEach(id => {
    const el = $(id);
    if (el) el.onchange = () => {
      scheduleSave();
      renderQueue();
    };
  });

  // 10. 初始化状态与数据加载
  try {
    const st = await api('/api/status');
    fillForm(st.config);
    if (st.unityExes) fillUnitySelect(st.unityExes);
    state.outputs = Array.isArray(st.outputs) ? st.outputs : [];
    renderOutputs();
    // 版本管理配置恢复
    const vc = (st.config && st.config.vcs) || {};
    state.vcsNodes = Array.isArray(vc.nodes)
      ? vc.nodes.map(n => ({ id: String(n.id || ''), name: n.name || '', path: n.path || '', type: n.type || 'auto' }))
      : [];
    state.vcsGroups = Array.isArray(vc.groups)
      ? vc.groups
          .filter(g => g && (g.id || g.name))
          .map(g => ({ id: String(g.id || ''), name: g.name || '分组', nodeIds: Array.isArray(g.nodeIds) ? g.nodeIds.map(String) : [] }))
      : [];
    renderVcs();
    if (state.vcsNodes.length) doVcsProbe(state.vcsNodes.map(n => n.id));
    if ($('projectPath').value.trim()) {
      doScan();
    }
  } catch (e) {
    console.error('init error:', e);
  }

  initSSE();
}

document.addEventListener('DOMContentLoaded', init);

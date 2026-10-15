/* Read-only multi-project monitoring. */
(() => {
  let selected = '', snapshot, channel = 'all', query = '', follow = true;
  const history = new Map();
  let artifactRoot = '', artifactPath = '', artifactRequest = 0;
  const artifactApi = (project, action, body) => api('/api/artifacts/'+action+'?project='+encodeURIComponent(project), body, true);
  function artifactSize(size) {
    if (size < 1024) return size+' B';
    if (size < 1048576) return (size/1024).toFixed(1)+' KB';
    return bytes(size);
  }
  async function artifacts(root = artifactRoot, path = artifactPath) {
    const project = selected, request = ++artifactRequest, host = $('monitorArtifacts');
    if (!project || !host) return;
    host.innerHTML = '<h4>产物预览</h4><p role="status">正在读取产物…</p>';
    try {
      const data = await artifactApi(project, 'list', {root, path});
      if (selected !== project || request !== artifactRequest) return;
      artifactRoot = data.root; artifactPath = data.path;
      host.innerHTML = `<div class="monitor-artifact-heading"><h4>产物预览</h4><button id="monitorArtifactRefresh" class="btn btn-ghost btn-xs">刷新</button></div>${data.roots.length ? `<div class="monitor-artifact-toolbar"><select id="monitorArtifactRoot" class="input-select" aria-label="产物目录">${data.roots.map(r=>`<option value="${esc(r.id)}" ${r.id===data.root?'selected':''}>${esc(r.name)}</option>`).join('')}</select><button id="monitorArtifactUp" class="btn btn-ghost btn-xs" ${data.path?'':'disabled'}>上一级</button><button id="monitorArtifactOpen" class="btn btn-ghost btn-xs">打开当前文件夹</button></div><p class="monitor-artifact-path">/${esc(data.path)}</p><div class="monitor-artifact-list">${data.entries.map((entry,i)=>`<div class="monitor-artifact-entry"><div>${entry.type==='directory'?`<button class="btn btn-ghost monitor-artifact-name" data-artifact-dir="${i}">📁 ${esc(entry.name)}</button>`:`<span class="monitor-artifact-name">📄 ${esc(entry.name)}</span>`}<small>${entry.type==='directory'?'文件夹':artifactSize(entry.size)} · ${entry.modified?esc(new Date(entry.modified).toLocaleString()):'时间未知'}</small></div><button class="btn btn-ghost btn-xs" data-artifact-open="${i}">${entry.type==='directory'?'打开文件夹':'打开所在文件夹'}</button></div>`).join('') || '<p class="field-hint">暂无产物，目录为空或尚未生成。</p>'}</div>` : '<p class="field-hint">尚未配置产物目录，请先设置工程路径。</p>'}`;
      $('monitorArtifactRefresh').onclick=()=>artifacts();
      if (!data.roots.length) return;
      $('monitorArtifactRoot').onchange=e=>artifacts(e.target.value,'');
      $('monitorArtifactUp').onclick=()=>artifacts(artifactRoot,artifactPath.split('/').slice(0,-1).join('/'));
      const open = async (path, button) => {
        button.disabled = true;
        try { await artifactApi(project,'open-folder',{root:data.root,path}); }
        catch(e) { showToast('打开失败：'+e.message,'err'); }
        finally { button.disabled = false; }
      };
      $('monitorArtifactOpen').onclick=e=>open(data.path,e.currentTarget);
      host.querySelectorAll('[data-artifact-dir]').forEach(b=>b.onclick=()=>artifacts(data.root,data.entries[Number(b.dataset.artifactDir)].path));
      host.querySelectorAll('[data-artifact-open]').forEach(b=>b.onclick=()=>open(data.entries[Number(b.dataset.artifactOpen)].path,b));
    } catch(e) {
      if (selected !== project || request !== artifactRequest) return;
      host.innerHTML = `<h4>产物预览</h4><p role="alert">读取失败：${esc(e.message)}</p><button id="monitorArtifactRetry" class="btn btn-ghost btn-xs">重试</button><button id="monitorArtifactReset" class="btn btn-ghost btn-xs">返回产物根目录</button>`;
      $('monitorArtifactRetry').onclick=()=>artifacts(root,path);
      $('monitorArtifactReset').onclick=()=>artifacts('','');
    }
  }
  const bytes = n => { n = Number(n) || 0; return n >= 1073741824 ? (n / 1073741824).toFixed(2) + ' GB' : (n / 1048576).toFixed(1) + ' MB'; };
  const tasks = p => [['构建', p.job], ['环境检测', p.check], ['仓库更新', p.vcs]].filter(([, t]) => t?.state);
  const current = p => tasks(p).find(([, t]) => ['running','starting','queued'].includes(t.state)) || tasks(p)[0];
  const names = {running:'运行中',starting:'启动中',queued:'排队中',done:'已结束',failed:'失败',stopped:'已停止',cancelled:'已取消',idle:'空闲'};
  function label(p) {
    if (!p.busy) return p.enabled === false ? '已停用' : '空闲';
    return names[current(p)?.[1].state] || '运行中';
  }
  function sample(id, r) {
    const values = history.get(id) || [];
    values.push({cpu:Number(r.cpuPercent)||0, memory:Number(r.memoryBytes)||0});
    if(values.length > 60) values.shift();
    history.set(id, values);
  }
  function chart(id, key, title, value) {
    const values = history.get(id) || [];
    const ceiling = key === 'cpu' ? 100 : Math.max(1048576, ...values.map(v=>v[key]));
    const points = values.map((v,i)=>`${(i / 59 * 300).toFixed(1)},${(66 - Math.min(1,v[key]/ceiling)*60).toFixed(1)}`).join(' ');
    return `<div class="monitor-chart"><div><span>${title}</span><strong>${value}</strong></div><svg viewBox="0 0 300 72" role="img" aria-label="${title}最近两分钟趋势"><path d="M0 6H300 M0 36H300 M0 66H300" class="chart-grid"/><polyline points="${points}" fill="none" stroke="currentColor" stroke-width="2"/></svg><small>最近两分钟 · ${key==='cpu'?'0–100%':'上限 '+bytes(ceiling)}</small></div>`;
  }
  function performance(id, r) {
    return `<div class="monitor-charts">${chart(id,'cpu','CPU',(Number(r.cpuPercent)||0).toFixed(1)+'%')}${chart(id,'memory','内存（进程树）',bytes(r.memoryBytes))}</div><p class="field-hint">${r.processCount||0} 个进程 · 磁盘读取 ${bytes(r.readBytes)} · 写入 ${bytes(r.writtenBytes)}</p>`;
  }
  function close() {
    ++artifactRequest;
    selected = ''; $('monitorDetail').classList.add('hidden'); $('monitorBackdrop').classList.add('hidden');
    document.querySelector(`[data-monitor-id="${CSS.escape(lastOpened)}"]`)?.focus();
  }
  let lastOpened = '';
  function logs() {
    const p = snapshot?.projects.find(p=>p.id===selected), el = $('monitorLogs');
    if(!p || !el) return;
    const scroll = el.scrollTop;
    el.textContent = (p.logs||[]).filter(l=>(channel==='all'||(channel==='build'?!/^(check|vcs)-/.test(l.type):l.type.startsWith(channel+'-'))) && (!query||l.text.toLowerCase().includes(query.toLowerCase()))).map(l=>`[${l.time}] ${l.text}`).join('\n') || '暂无日志';
    el.scrollTop = follow ? el.scrollHeight : scroll;
  }
  function detail() {
    const p = snapshot?.projects.find(p=>p.id===selected);
    if(!p) { if(selected) close(); return; }
    const panel = $('monitorDetail');
    if(panel.dataset.project !== selected) {
      panel.dataset.project = selected;
      panel.innerHTML = `<div class="card-header"><h3 id="monitorTitle"></h3><button id="monitorClose" class="btn btn-ghost" aria-label="关闭项目详情">×</button></div><div class="monitor-drawer-body"><div id="monitorPerformance"></div><div id="monitorTask"></div><section id="monitorArtifacts" class="monitor-artifacts" aria-label="产物预览"></section><div class="monitor-website"><label for="monitorWebsiteUrl">项目用户网页</label><div class="monitor-website-row"><input id="monitorWebsiteUrl" class="input-text" type="text" readonly aria-label="项目用户网页地址，可选中复制"><button id="monitorOpenWebsite" type="button" class="btn btn-primary">打开网页</button></div></div><div class="monitor-log-toolbar"><select id="monitorLogChannel" class="input-select"><option value="all">全部日志</option><option value="build">构建日志</option><option value="check">检测日志</option><option value="vcs">仓库日志</option></select><input id="monitorLogQuery" class="input-text" placeholder="搜索日志"><label><input id="monitorLogFollow" type="checkbox" checked> 自动滚动</label><button id="monitorCopyLogs" class="btn btn-ghost btn-xs">复制</button></div><pre id="monitorLogs" class="terminal-body"></pre><p class="field-hint">保留本次运行最近 300 条输出。项目占用统计当前任务及子进程；空闲时任务进程占用为 0。CPU 按整机总处理能力计算。</p></div>`;
      $('monitorClose').onclick=close;
      $('monitorLogChannel').onchange=e=>{channel=e.target.value;logs();};
      $('monitorLogQuery').oninput=e=>{query=e.target.value;logs();};
      $('monitorLogFollow').onchange=e=>{follow=e.target.checked;logs();};
      $('monitorCopyLogs').onclick=()=>copyToClipboard($('monitorLogs').textContent);
      artifactRoot = ''; artifactPath = ''; artifacts();
    }
    $('monitorTitle').textContent=p.name+' · '+label(p);
    const websiteUrl = new URL('/p/'+encodeURIComponent(p.slug), location.origin);
    if ($('monitorWebsiteUrl').value !== websiteUrl.href) $('monitorWebsiteUrl').value = websiteUrl.href;
    $('monitorOpenWebsite').onclick = () => api('/api/open-user-page', {url: websiteUrl.href}, true).catch(e => showToast('打开失败：'+e.message, 'err'));
    $('monitorPerformance').innerHTML=performance(p.id,p.resources);
    const [type, task] = current(p)||['构建',{}], queue=task.queue||task.targets||task.nodes||[];
    const progress=task.progress?.[String(task.index)]||task.progress||{};
    const pct=Number(progress.pct??progress.percent), percent=Number.isFinite(pct)?Math.max(0,Math.min(100,pct)):null;
    $('monitorTask').innerHTML=`<h4>任务进度</h4><p>${esc(type)} · ${esc(names[task.state]||'尚未执行')}${task.end?.ok===false?' · 最近任务失败或取消':''}</p><p>${esc(progress.stage||progress.message||'等待任务')} ${percent===null?'':percent.toFixed(1)+'%'}</p>${percent===null?'':`<progress max="100" value="${percent}"></progress>`}<p>共 ${queue.length} 个节点 · 当前节点 ${task.state==='running'&&task.index>=0?task.index+1:'—'}</p>${tasks(p).map(([n,t])=>`<p>${esc(n)}：${esc(names[t.state]||t.state)}${t.end?.ok===false?' · 未成功':''}</p>`).join('')}`;
    panel.classList.remove('hidden'); $('monitorBackdrop').classList.remove('hidden'); logs();
  }
  function render() {
    $('monitorHost').innerHTML=`<div class="monitor-total-heading">整个程序运行占用 · ${snapshot.projects.length} 个项目 · ${snapshot.scheduler.pendingProjectIds.length} 个排队</div>${performance('backend',snapshot.backend)}<p class="field-hint">主机内存 ${bytes(snapshot.hostMemoryUsed)} / ${bytes(snapshot.hostMemoryTotal)}</p>`;
    $('monitorCards').innerHTML=snapshot.projects.map(p=>`<button class="monitor-project-card ${p.id===selected?'active':''}" data-monitor-id="${esc(p.id)}"><strong>${esc(p.name)}</strong><span class="monitor-state">${esc(label(p))}</span><small>/p/${esc(p.slug)}</small>${performance(p.id,p.resources)}<span>查看详细监控 →</span></button>`).join('')||'<p class="empty-state">暂无项目，请在设置中添加项目。</p>';
    $('monitorCards').querySelectorAll('[data-monitor-id]').forEach(b=>b.onclick=()=>{selected=b.dataset.monitorId;lastOpened=selected;channel='all';query='';follow=true;$('monitorDetail').dataset.project='';detail();$('monitorClose').focus();});
    detail();
  }
  const backdrop=document.createElement('div');backdrop.id='monitorBackdrop';backdrop.className='monitor-backdrop hidden';document.body.append(backdrop);backdrop.onclick=close;
  const panel=$('monitorDetail');document.body.append(panel);panel.classList.add('monitor-drawer');panel.setAttribute('role','dialog');panel.setAttribute('aria-modal','true');panel.setAttribute('aria-labelledby','monitorTitle');
  document.addEventListener('keydown',e=>{if(!selected)return;if(e.key==='Escape'){close();return;}if(e.key==='Tab'){const list=[...panel.querySelectorAll('button,input,select,a[href]')];const first=list[0],last=list.at(-1);if(e.shiftKey&&document.activeElement===first){e.preventDefault();last.focus();}else if(!e.shiftKey&&document.activeElement===last){e.preventDefault();first.focus();}}});
  async function poll(){try{snapshot=await api('/api/monitor',undefined,true);sample('backend',snapshot.backend);snapshot.projects.forEach(p=>sample(p.id,p.resources));render();}catch(e){$('monitorHost').textContent='状态采集失败：'+e.message;}finally{setTimeout(poll,2000);}}
  poll();
})();

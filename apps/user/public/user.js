"use strict";
(() => {
  const $ = (id) => document.getElementById(id);
  const route = window.location.pathname.match(/^\/p\/([^/]+)(?:\/workspace)?\/?$/);
  let slug = "";
  try { slug = route ? decodeURIComponent(route[1]) : ""; } catch (_) {}
  const projectHome = slug ? `/p/${encodeURIComponent(slug)}` : "";
  const apiBase = slug ? `/api/projects/${encodeURIComponent(slug)}` : "";
  document.querySelectorAll("[data-project-home]").forEach((link) => {
    link.href = projectHome || "#";
  });
  document.querySelectorAll("[data-project-workspace]").forEach((link) => {
    link.href = projectHome ? `${projectHome}/workspace` : "#";
  });
  function setProject(project) {
    const name = project && (project.name || project.slug) || slug;
    document.querySelectorAll("[data-project-name]").forEach((el) => {
      el.textContent = name;
    });
    document.title = `${name} · ${document.body.dataset.page === "workspace" ? "构建工作台" : "构建首页"} · TBuildTool`;
  }
  function projectError(message) {
    notice(message);
    document.querySelectorAll("[data-project-workspace]").forEach((link) => {
      link.removeAttribute("href");
      link.setAttribute("aria-disabled", "true");
    });
  }
  if (!slug) {
    projectError("无效的项目链接，请使用管理员提供的 /p/项目标识 链接访问。");
    return;
  }
  if (document.body.dataset.page !== "workspace") {
    api(`${apiBase}/catalog`).then((data) => setProject(data.project))
      .catch((error) => projectError(`项目加载失败：${error.message}`));
    return;
  }
  const state = {
    nodes: [],
    groups: [],
    selected: new Set(),
    job: null,
    busy: false,
    queued: false,
    queuePosition: null,
    queueEnabled: true,
    projectReady: false,
    submitting: false,
    stopping: false,
    connected: false,
    statusKnown: false,
    queueStatus: {},
    logs: [],
    roots: [],
    root: "",
    path: "",
    entries: [],
    fileSelection: new Set(),
    fileRequest: 0,
    filesLoading: false,
    catalogLoading: false,
    statusRequest: null,
  };
  const stages = {
    prepare: "准备构建",
    preparing: "准备构建",
    build: "构建中",
    building: "构建中",
    unity: "Unity 构建中",
    package: "打包中",
    packaging: "打包中",
    upload: "上传中",
    finish: "完成",
    done: "完成",
  };
  function element(tag, className, text) {
    const el = document.createElement(tag);
    if (className) el.className = className;
    if (text !== undefined) el.textContent = text;
    return el;
  }
  function notice(message) {
    $("notice").textContent = message || "";
    $("notice").hidden = !message;
  }
  async function api(url, body) {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 20000);
    try {
      const response = await fetch(url, {
        cache: "no-store",
        signal: controller.signal,
        ...(body !== undefined
          ? {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify(body),
            }
          : {}),
      });
      const data = await response.json().catch(() => null);
      if (!response.ok) {
        if (response.status === 404) throw new Error("项目不存在或已停用，请联系管理员检查项目链接。");
        throw new Error(
          data && data.error
            ? String(data.error)
            : `请求失败（${response.status}）`,
        );
      }
      if (data === null) throw new Error("服务器返回了无法读取的数据");
      return data;
    } catch (error) {
      if (error.name === "AbortError") throw new Error("请求超时，请稍后重试");
      throw error;
    } finally {
      clearTimeout(timeout);
    }
  }
  function hasActiveBuild() {
    return (
      !!state.job &&
      ["starting", "running"].includes(state.job.state) &&
      !state.job.end
    );
  }
  function canSubmit() {
    return state.projectReady && !state.queued &&
      !hasActiveBuild() && (!state.busy || state.queueEnabled);
  }
  function renderSelection() {
    const blocked = !canSubmit() || state.submitting || state.catalogLoading;
    $("selected-count").textContent = `已选 ${state.selected.size} 个目标`;
    $("build").disabled = blocked || !state.statusKnown || !state.selected.size;
    $("build").textContent = state.submitting
      ? "正在提交…"
      : state.queued
        ? "任务已排队"
        : state.busy
          ? hasActiveBuild()
            ? "构建正在运行"
            : state.queueEnabled ? "加入构建队列 ↗" : "其他任务运行中"
          : "开始构建 ↗";
    $("select-all").disabled = blocked || !state.nodes.length;
    $("select-all").textContent =
      state.nodes.length && state.selected.size === state.nodes.length
        ? "取消全选"
        : "全选";
    $("catalog-refresh").disabled = state.catalogLoading;
    $("catalog")
      .querySelectorAll("input")
      .forEach((input) => {
        input.checked = state.selected.has(input.value);
        input.disabled = blocked;
      });
    $("groups")
      .querySelectorAll("button")
      .forEach((button) => {
        button.disabled = blocked;
      });
  }
  function renderCatalog() {
    $("catalog").replaceChildren();
    $("groups").replaceChildren();
    state.groups.forEach((group) => {
      const button = element(
        "button",
        "group-button",
        group.name || "未命名分组",
      );
      button.type = "button";
      button.addEventListener("click", () => {
        const ids = (group.nodeIds || []).filter((id) =>
          state.nodes.some((node) => node.id === id),
        );
        const all = ids.length && ids.every((id) => state.selected.has(id));
        ids.forEach((id) =>
          all ? state.selected.delete(id) : state.selected.add(id),
        );
        renderSelection();
      });
      $("groups").append(button);
    });
    if (!state.nodes.length)
      $("catalog").append(
        element("p", "empty-state", "暂无可用构建配置，请联系管理员添加。"),
      );
    state.nodes.forEach((node) => {
      const card = element("label", "node-card");
      const checkbox = element("input");
      checkbox.type = "checkbox";
      checkbox.value = node.id;
      checkbox.setAttribute("aria-label", `选择 ${node.name}`);
      checkbox.addEventListener("change", () => {
        if (checkbox.checked) state.selected.add(node.id);
        else state.selected.delete(node.id);
        renderSelection();
      });
      const info = element("div", "node-info");
      info.append(element("div", "node-name", node.name || "未命名配置"));
      const meta = element("div", "node-meta");
      meta.append(
        element("span", "", node.targetName || "默认目标"),
        element(
          "span",
          "",
          node.bundleVersion ? `v${node.bundleVersion}` : "版本未设置",
        ),
      );
      info.append(meta);
      card.append(checkbox, info);
      if (node.dev) card.append(element("span", "dev-tag", "开发版"));
      $("catalog").append(card);
    });
    renderSelection();
  }
  async function loadCatalog() {
    state.catalogLoading = true;
    renderSelection();
    try {
      const data = await api(`${apiBase}/catalog`);
      state.projectReady = true;
      setProject(data.project);
      state.nodes = Array.isArray(data.nodes) ? data.nodes : [];
      state.groups = Array.isArray(data.groups) ? data.groups : [];
      state.selected = new Set(
        [...state.selected].filter((id) =>
          state.nodes.some((node) => node.id === id),
        ),
      );
      renderCatalog();
      await refreshStatus(true);
      loadFiles();
      if (!source) connectEvents();
    } catch (error) {
      state.projectReady = false;
      notice(`配置加载失败：${error.message}`);
      if (!state.nodes.length) {
        $("catalog").replaceChildren(
          element("p", "empty-state", "无法加载配置，点击刷新重试。"),
        );
      }
    } finally {
      state.catalogLoading = false;
      renderSelection();
    }
  }
  function renderJob() {
    const job = state.job;
    const active = hasActiveBuild();
    const adminBusy = state.busy && !active && !state.queued;
    $("stop").hidden = !active && !state.queued;
    $("stop").disabled = state.stopping;
    $("stop").textContent = state.stopping ? "正在停止…" : state.queued ? "取消排队" : "停止构建";
    const ended = job && job.end;
    const index = Math.max(0, Number(job && job.index) || 0);
    const count = Number(job && job.count) || ((job && job.queue) || []).length;
    const progress = (job && job.progress && job.progress[index]) || {};
    const pct = Math.max(0, Math.min(100, Number(progress.pct) || 0));
    const overall =
      ended && ended.ok
        ? 100
        : count
          ? Math.min(100, ((index + pct / 100) / count) * 100)
          : 0;
    $("job-state").textContent = state.queued
      ? "等待构建"
      : adminBusy
        ? "其他任务运行中"
      : active
        ? "构建中"
        : ended
          ? ended.ok
            ? "已完成"
            : ended.reason === "stopped" || ended.reason === "cancelled"
              ? "已停止"
              : "未完成"
          : "待命";
    const current = job && job.queue && job.queue[index];
    $("job-title").textContent = state.queued
      ? "任务已提交，等待轮到当前项目"
      : adminBusy
        ? "其他任务正在运行，可等待队列调度"
      : active
        ? current
          ? current.name
          : "构建任务运行中"
        : ended
          ? ended.ok
            ? "构建任务已完成"
            : "构建任务已结束"
          : "准备好，开启一次新构建";
    $("job-percent").textContent =
      !adminBusy && job ? `${Math.round(overall)}%` : "—";
    $("progress-bar").style.width = `${adminBusy ? 0 : overall}%`;
    $("job-stage").textContent = state.queued
      ? `已进入队列${state.queuePosition != null ? ` · 当前排在第 ${state.queuePosition} 位` : ""}`
      : adminBusy
      ? "等待管理员任务完成后，即可开始构建。"
      : active
        ? `${count ? `${Math.min(index + 1, count)} / ${count} · ` : ""}${stages[progress.stage] || progress.stage || "等待进度更新"}`
        : ended
          ? ended.message ||
            (ended.ok ? "所有构建已完成，可在下方下载产物" : "查看日志了解详情")
          : "选择左侧目标后开始";
    $("queue").replaceChildren();
    ((job && job.queue) || []).forEach((item, i) => {
      const result = state.queueStatus[i];
      const status =
        result === false
          ? "failed"
          : result === true || i < index || (ended && ended.ok)
            ? "success"
            : active && i === index
              ? "current"
              : "";
      const row = element("div", `queue-item ${status}`);
      row.append(
        element(
          "span",
          "queue-index",
          status === "success"
            ? "✓"
            : status === "failed"
              ? "×"
              : String(i + 1),
        ),
        element("span", "queue-name", item.name || "构建目标"),
        element("span", "queue-target", item.targetName || item.target || ""),
      );
      $("queue").append(row);
    });
    $("job-result").hidden = !ended;
    if (ended) {
      $("job-result").classList.toggle("error", !ended.ok);
      $("job-result").textContent =
        ended.message ||
        (ended.ok
          ? "构建完成。产物目录已刷新。"
          : `构建未完成${ended.reason ? `：${ended.reason}` : "，请查看日志或重试。"}`);
    }
    renderSelection();
  }
  function appendLog(text) {
    if (!text) return;
    const logs = $("logs");
    const nearBottom =
      logs.scrollHeight - logs.scrollTop - logs.clientHeight < 45;
    state.logs.push(String(text));
    if (state.logs.length > 800) state.logs.splice(0, state.logs.length - 800);
    logs.textContent = state.logs.join("\n");
    if (nearBottom) logs.scrollTop = logs.scrollHeight;
  }
  function applyStatus(data) {
    state.statusKnown = true;
    const wasBusy = state.busy;
    state.job = data.job || null;
    state.busy = !!data.busy;
    const jobState = String(data.job && data.job.state || data.state || "").toLowerCase();
    state.queued = jobState === "queued" || jobState === "pending" || !!data.queued;
    state.queuePosition = data.queuePosition ?? (data.job && (data.job.queuePosition ?? data.job.position));
    state.queueEnabled = data.queueEnabled !== false && data.allowQueue !== false;
    renderJob();
    if (wasBusy && !state.busy) loadFiles(state.root, state.path);
  }
  async function refreshStatus(showError = false) {
    if (!state.projectReady) return;
    if (state.statusRequest) return state.statusRequest;
    state.statusRequest = (async () => {
      try {
        applyStatus(await api(`${apiBase}/status`));
      } catch (error) {
        if (showError) notice(`状态恢复失败：${error.message}`);
      } finally {
        state.statusRequest = null;
      }
    })();
    return state.statusRequest;
  }
  async function startBuild() {
    if (!canSubmit() || state.submitting || !state.selected.size) return;
    state.submitting = true;
    notice("");
    renderSelection();
    try {
      await api(`${apiBase}/build`, { nodeIds: [...state.selected] });
      state.busy = true;
      state.queueStatus = {};
      state.logs = [];
      $("logs").textContent = "任务已提交，等待实时进度…";
      renderJob();
      await refreshStatus(true);
    } catch (error) {
      notice(`启动失败：${error.message}`);
      await refreshStatus();
    } finally {
      state.submitting = false;
      renderSelection();
    }
  }
  async function stopBuild() {
    if (state.stopping || (!hasActiveBuild() && !state.queued)) return;
    state.stopping = true;
    renderJob();
    try {
      await api(`${apiBase}/build/stop`, {});
      appendLog("已请求停止构建，正在等待任务结束…");
      await refreshStatus(true);
    } catch (error) {
      notice(`停止失败：${error.message}`);
    } finally {
      state.stopping = false;
      renderJob();
    }
  }
  function handleEvent(type, data) {
    if (!data || typeof data !== "object") data = {};
    if (type === "hello") {
      refreshStatus(true);
      return;
    }
    if (type === "queued" || type === "queue" || type === "job-queued") {
      refreshStatus();
      return;
    }
    if (type === "job-start") {
      state.queued = false;
      state.queueStatus = {};
      state.logs = [];
      $("logs").textContent = "构建开始…";
      state.busy = true;
      state.job = {
        state: "running",
        index: 0,
        count: data.count,
        queue: [...state.selected]
          .map((id) => state.nodes.find((node) => node.id === id))
          .filter(Boolean),
        progress: {},
      };
      refreshStatus();
    }
    if (type === "profile-start") {
      state.busy = true;
      state.job = state.job || { queue: [], progress: {}, count: 0 };
      state.job.index = Number(data.index) || 0;
      state.job.queue = state.job.queue || [];
      state.job.queue[state.job.index] = {
        name: data.name,
        targetName: data.targetName || data.target,
      };
      appendLog(
        `开始构建：${data.name || "目标"}${data.target ? ` / ${data.target}` : ""}`,
      );
    }
    if (type === "progress" && state.job) {
      const index = Number(data.index) || 0;
      state.job.index = index;
      state.job.progress = state.job.progress || {};
      state.job.progress[index] = { stage: data.stage, pct: data.pct };
    }
    if (type === "profile-end") {
      state.queueStatus[data.index] = !!data.ok;
      appendLog(
        `目标 ${Number(data.index) + 1} ${data.ok ? "构建完成" : "构建失败"}`,
      );
    }
    if (type === "job-end" || type === "done") {
      const end = data.end || data;
      state.busy = false;
      state.job = state.job || { queue: [], progress: {} };
      state.job.end = end;
      state.job.state = "done";
      appendLog(end.message || (end.ok ? "构建任务完成" : "构建任务结束"));
      loadFiles(state.root, state.path);
      refreshStatus();
    }
    if (type === "output") {
      appendLog(`产物已就绪：${data.name || data.targetName || "构建产物"}`);
      loadFiles(state.root, state.path);
    }
    renderJob();
  }
  let source;
  function connectEvents() {
    if (!window.EventSource) {
      $("connection-label").textContent = "使用定时刷新";
      $("connection-dot").classList.add("offline");
      return;
    }
    source = new EventSource(`${apiBase}/events`);
    source.onopen = () => {
      state.connected = true;
      $("connection-label").textContent = "实时连接";
      $("connection-dot").classList.remove("offline");
      refreshStatus(true);
    };
    source.onerror = () => {
      state.connected = false;
      $("connection-label").textContent = "连接中断 · 自动重连";
      $("connection-dot").classList.add("offline");
      refreshStatus();
    };
    const receive = (type) => (event) => {
      try {
        const data = JSON.parse(event.data);
        handleEvent(
          type || data.type || data.event,
          data.data && typeof data.data === "object" ? data.data : data,
        );
      } catch (_) {
        /* Ignore incomplete event frames; status polling recovers state. */
      }
    };
    [
      "hello",
      "queued",
      "queue",
      "job-queued",
      "job-start",
      "profile-start",
      "progress",
      "profile-end",
      "job-end",
      "done",
      "output",
    ].forEach((type) => source.addEventListener(type, receive(type)));
    source.onmessage = receive();
  }
  function sizeLabel(value) {
    if (!Number.isFinite(Number(value))) return "—";
    const size = Number(value);
    if (size < 1024) return `${size} B`;
    if (size < 1048576) return `${(size / 1024).toFixed(1)} KB`;
    if (size < 1073741824) return `${(size / 1048576).toFixed(1)} MB`;
    return `${(size / 1073741824).toFixed(1)} GB`;
  }
  function dateLabel(value) {
    if (!value) return "—";
    const date = new Date(value);
    return Number.isNaN(date.getTime())
      ? "—"
      : date.toLocaleString("zh-CN", { hour12: false });
  }
  function updateFileSelection() {
    $("download").disabled = state.filesLoading || !state.fileSelection.size;
    $("file-selected-count").textContent = `(${state.fileSelection.size})`;
    $("file-select-all").disabled = state.filesLoading || !state.entries.length;
    $("file-select-all").checked =
      !!state.entries.length &&
      state.fileSelection.size === state.entries.length;
    $("file-select-all").indeterminate =
      state.fileSelection.size > 0 &&
      state.fileSelection.size < state.entries.length;
  }
  async function downloadEntry(entry, root = state.root) {
    try {
      const params = new URLSearchParams({ root, path: entry.path });
      const downloadUrl = `${apiBase}/${entry.type === "directory" ? "download-folder" : "download"}?${params}`;
      // Stream ZIPs, large files, and unknown sizes through the browser download manager.
      const knownSize =
        typeof entry.size === "number" && Number.isFinite(entry.size);
      if (
        entry.type === "directory" ||
        !knownSize ||
        entry.size > 16 * 1024 * 1024
      ) {
        const anchor = element("a");
        anchor.href = downloadUrl;
        anchor.download =
          entry.type === "directory" ? `${entry.name}.zip` : entry.name;
        document.body.append(anchor);
        anchor.click();
        anchor.remove();
        return true;
      }
      const response = await fetch(downloadUrl);
      if (!response.ok) {
        const error = await response.json().catch(() => null);
        throw new Error(
          (error && error.error) || `请求失败（${response.status}）`,
        );
      }
      const blob = await response.blob();
      const url = URL.createObjectURL(blob);
      const anchor = element("a");
      anchor.href = url;
      anchor.download =
        entry.type === "directory" ? `${entry.name}.zip` : entry.name;
      document.body.append(anchor);
      anchor.click();
      anchor.remove();
      setTimeout(() => URL.revokeObjectURL(url), 60000);
      return true;
    } catch (error) {
      $("file-status").textContent =
        `下载 ${entry.name} 失败：${error.message}`;
      return false;
    }
  }
  function renderFiles() {
    const select = $("root-select");
    select.replaceChildren();
    if (!state.roots.length) {
      const option = element("option", "", "暂无产物目录");
      option.value = "";
      select.append(option);
    }
    state.roots.forEach((root) => {
      const option = element("option", "", root.name || root.id);
      option.value = root.id;
      select.append(option);
    });
    select.value = state.root;
    const breadcrumbs = $("breadcrumbs");
    breadcrumbs.replaceChildren();
    if (state.root) {
      const root = state.roots.find((item) => item.id === state.root);
      const button = element("button", "", root ? root.name : "根目录");
      button.type = "button";
      button.addEventListener("click", () => loadFiles(state.root, ""));
      breadcrumbs.append(button);
      let relative = "";
      state.path
        .split("/")
        .filter(Boolean)
        .forEach((part) => {
          relative += `${relative ? "/" : ""}${part}`;
          const path = relative;
          breadcrumbs.append(element("span", "", "/"));
          const crumb = element("button", "", part);
          crumb.type = "button";
          crumb.addEventListener("click", () => loadFiles(state.root, path));
          breadcrumbs.append(crumb);
        });
    }
    $("parent").disabled = !state.path || state.filesLoading;
    $("files").replaceChildren();
    if (!state.entries.length) {
      const row = element("tr");
      const cell = element(
        "td",
        "empty-state",
        state.roots.length
          ? "这个目录还是空的，构建完成后可刷新查看。"
          : "暂无可用产物目录，请联系管理员配置。",
      );
      cell.colSpan = 5;
      row.append(cell);
      $("files").append(row);
    }
    state.entries.forEach((entry) => {
      const row = element("tr");
      const checkCell = element("td");
      const check = element("input");
      check.type = "checkbox";
      check.checked = state.fileSelection.has(entry.path);
      check.setAttribute("aria-label", `选择 ${entry.name}`);
      check.addEventListener("change", () => {
        check.checked
          ? state.fileSelection.add(entry.path)
          : state.fileSelection.delete(entry.path);
        updateFileSelection();
      });
      checkCell.append(check);
      const nameCell = element("td");
      const name = element(
        entry.type === "directory" ? "button" : "span",
        "file-name",
      );
      name.append(
        element("span", "file-icon", entry.type === "directory" ? "▱" : "▫"),
        document.createTextNode(entry.name),
      );
      if (entry.type === "directory") {
        name.type = "button";
        name.setAttribute("aria-label", `打开目录 ${entry.name}`);
        name.addEventListener("click", () => loadFiles(state.root, entry.path));
      }
      nameCell.append(name);
      const actionCell = element("td");
      const download = element(
        "button",
        "text-button",
        entry.type === "directory" ? "下载 ZIP" : "下载",
      );
      download.type = "button";
      download.addEventListener("click", () => downloadEntry(entry));
      actionCell.append(download);
      row.append(
        checkCell,
        nameCell,
        element(
          "td",
          "",
          entry.type === "directory" ? "—" : sizeLabel(entry.size),
        ),
        element("td", "", dateLabel(entry.modified)),
        actionCell,
      );
      $("files").append(row);
    });
    updateFileSelection();
  }
  async function loadFiles(root = "", path = "") {
    const request = ++state.fileRequest;
    state.filesLoading = true;
    $("files-refresh").disabled = true;
    $("root-select").disabled = true;
    $("parent").disabled = true;
    $("file-status").textContent = "正在读取目录…";
    updateFileSelection();
    try {
      const params = new URLSearchParams();
      if (root) params.set("root", root);
      if (path) params.set("path", path);
      const data = await api(`${apiBase}/files?${params}`);
      if (request !== state.fileRequest) return;
      state.roots = Array.isArray(data.roots) ? data.roots : [];
      state.root = data.root || "";
      state.path = data.path || "";
      state.entries = (Array.isArray(data.entries) ? data.entries : []).sort(
        (a, b) =>
          (a.type === "directory" ? 0 : 1) - (b.type === "directory" ? 0 : 1) ||
          a.name.localeCompare(b.name, "zh-CN"),
      );
      state.fileSelection.clear();
      state.filesLoading = false;
      renderFiles();
      $("file-status").textContent = `${state.entries.length} 个项目`;
    } catch (error) {
      if (request !== state.fileRequest) return;
      $("file-status").textContent =
        `目录加载失败：${error.message}。请点击刷新重试。`;
      if (!state.root) {
        const row = element("tr");
        const cell = element("td", "empty-state", "产物目录加载失败");
        cell.colSpan = 5;
        row.append(cell);
        $("files").replaceChildren(row);
        $("root-select").replaceChildren(element("option", "", "加载失败"));
      }
    } finally {
      if (request === state.fileRequest) {
        state.filesLoading = false;
        $("files-refresh").disabled = false;
        $("root-select").disabled = false;
        $("parent").disabled = !state.path;
        updateFileSelection();
      }
    }
  }
  $("catalog-refresh").addEventListener("click", () => {
    notice("");
    loadCatalog();
  });
  $("select-all").addEventListener("click", () => {
    state.selected =
      state.selected.size === state.nodes.length
        ? new Set()
        : new Set(state.nodes.map((node) => node.id));
    renderSelection();
  });
  $("build").addEventListener("click", startBuild);
  $("stop").addEventListener("click", stopBuild);
  $("clear-log").addEventListener("click", () => {
    state.logs = [];
    $("logs").textContent = "日志显示已清空，后续日志将继续追加。";
  });
  $("files-refresh").addEventListener("click", () =>
    loadFiles(state.root, state.path),
  );
  $("root-select").addEventListener("change", (event) =>
    loadFiles(event.target.value, ""),
  );
  $("parent").addEventListener("click", () =>
    loadFiles(
      state.root,
      state.path.split("/").filter(Boolean).slice(0, -1).join("/"),
    ),
  );
  $("file-select-all").addEventListener("change", (event) => {
    state.fileSelection = event.target.checked
      ? new Set(state.entries.map((entry) => entry.path))
      : new Set();
    $("files")
      .querySelectorAll("input")
      .forEach((input) => {
        input.checked = event.target.checked;
      });
    updateFileSelection();
  });
  $("download").addEventListener("click", async () => {
    const entries = state.entries.filter((entry) =>
      state.fileSelection.has(entry.path),
    );
    const root = state.root;
    $("download").disabled = true;
    let failed = 0;
    for (let index = 0; index < entries.length; index++) {
      $("file-status").textContent =
        `正在下载 ${index + 1} / ${entries.length}：${entries[index].name}`;
      if (!(await downloadEntry(entries[index], root))) failed++;
    }
    $("file-status").textContent =
      `已处理 ${entries.length} 个下载${failed ? `，${failed} 个失败，请逐个重试` : "；若下载未保存，请允许浏览器多文件下载"}。`;
    updateFileSelection();
  });
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) refreshStatus(true);
  });
  loadCatalog();
  let poll = setInterval(() => {
    if (
      !document.hidden &&
      (!state.connected || state.busy || !state.statusKnown)
    )
      refreshStatus();
  }, 5000);
  window.addEventListener("pagehide", () => {
    clearInterval(poll);
    if (source) source.close();
  });
  window.addEventListener("pageshow", (event) => {
    if (event.persisted) {
      connectEvents();
      refreshStatus(true);
      loadFiles(state.root, state.path);
      poll = setInterval(() => {
        if (
          !document.hidden &&
          (!state.connected || state.busy || !state.statusKnown)
        )
          refreshStatus();
      }, 5000);
    }
  });
})();

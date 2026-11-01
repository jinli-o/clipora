"use strict";

const T = window.__TAURI__;
const invoke = T.core.invoke;
const listen = T.event.listen;

const $ = (s) => document.querySelector(s);
const root = document.documentElement;
const q = $("#q");
const list = $("#list");
const hl = $("#hl");

const IMG_BASE = "http://clip.localhost/";

const state = {
  items: [],
  view: [],
  filter: "all",
  sel: 0,
  settings: null,
  paused: false,
  dataDir: "",
  previewOpen: false,
  settingsOpen: false,
  hiding: false,
  fresh: null,
};

// ================= 内容类型识别 =================
const KIND = {
  text:  { label: "文本", color: "var(--c-text)",  icon: '<path d="M5 6h14M5 12h14M5 18h9"/>' },
  link:  { label: "链接", color: "var(--c-link)",  icon: '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>' },
  code:  { label: "代码", color: "var(--c-code)",  icon: '<path d="m8 7-5 5 5 5M16 7l5 5-5 5M13.5 4l-3 16"/>' },
  color: { label: "颜色", color: "var(--c-color)", icon: "" },
  image: { label: "图片", color: "var(--c-image)", icon: '<rect x="3" y="4" width="18" height="16" rx="3"/><circle cx="9" cy="10" r="2"/><path d="m21 16-5-5-9 9"/>' },
  files: { label: "文件", color: "var(--c-files)", icon: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"/>' },
  num:   { label: "数字", color: "var(--c-num)",   icon: '<path d="M9 4 7 20M17 4l-2 16M4 9h16M3 15h16"/>' },
  mail:  { label: "邮箱", color: "var(--c-mail)",  icon: '<circle cx="12" cy="12" r="4"/><path d="M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-4 8"/>' },
};

const reColor = /^(#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})|rgba?\([^)]{5,60}\)|hsla?\([^)]{5,60}\))$/i;
const reLink = /^(https?:\/\/|www\.)[^\s]+$/i;
const reMail = /^[^\s@]+@[^\s@]+\.[a-z]{2,}$/i;
const reNum = /^[-+]?(\d{1,3}(,\d{3})+|\d+)(\.\d+)?%?$/;
const reCodeKw = /\b(function|const|let|var|def|class|import|export|return|public|private|static|void|fn|impl|struct|SELECT|FROM|WHERE|#include|package|func|=>|console\.|print\()/;

function classify(it) {
  if (it._k) return it._k;
  let k = "text";
  if (it.kind === "image") k = "image";
  else if (it.kind === "files") k = "files";
  else {
    const t = it.text.trim();
    if (t.length < 80 && reColor.test(t) && CSS.supports("color", t)) k = "color";
    else if (t.length < 4000 && reLink.test(t)) k = "link";
    else if (t.length < 200 && reMail.test(t)) k = "mail";
    else if (t.length < 40 && reNum.test(t)) k = "num";
    else {
      let score = 0;
      if (reCodeKw.test(t)) score++;
      if (/[{};]\s*$/m.test(t)) score++;
      if (/^( {2,}|\t)\S/m.test(t)) score++;
      if (/<\/?[a-z][\w-]*[^>]*>/i.test(t) && /\n/.test(t)) score++;
      if (/[=(){}[\]<>]/.test(t) && /\n/.test(t)) score++;
      if (score >= 2) k = "code";
    }
  }
  it._k = k;
  return k;
}

// ================= 工具函数 =================
const esc = (s) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const svg = (path) => `<svg viewBox="0 0 24 24" class="ico">${path}</svg>`;
const ICON_PIN = '<path d="M12 17v5"/><path d="M9 3h6l-1 6 4 4v2H6v-2l4-4Z"/>';
const ICON_DEL = '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>';
const ICON_COPY = '<rect x="8" y="8" width="12" height="12" rx="2.5"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/>';
const ICON_PASTE = '<path d="M9 4h6v3H9z"/><path d="M15 5h2a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h2"/>';
const ICON_EYE = '<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/>';
const ICON_TXT = '<path d="M5 6h14M12 6v13"/>';

function ago(ms) {
  const d = Date.now() - ms;
  const m = Math.floor(d / 60000);
  if (m < 1) return "刚刚";
  if (m < 60) return `${m} 分钟前`;
  const h = Math.floor(m / 60);
  const dt = new Date(ms);
  const today = new Date(); today.setHours(0, 0, 0, 0);
  if (ms >= today.getTime()) return `${h} 小时前`;
  if (ms >= today.getTime() - 86400000) return `昨天 ${String(dt.getHours()).padStart(2, "0")}:${String(dt.getMinutes()).padStart(2, "0")}`;
  if (dt.getFullYear() === today.getFullYear()) return `${dt.getMonth() + 1} 月 ${dt.getDate()} 日`;
  return `${dt.getFullYear()}/${dt.getMonth() + 1}/${dt.getDate()}`;
}

const APP_NAMES = {
  chrome: "Chrome", msedge: "Edge", firefox: "Firefox", code: "VS Code", cursor: "Cursor",
  winword: "Word", excel: "Excel", powerpnt: "PowerPoint", onenote: "OneNote", outlook: "Outlook",
  explorer: "资源管理器", notepad: "记事本", wechat: "微信", weixin: "微信", qq: "QQ", dingtalk: "钉钉",
  feishu: "飞书", lark: "飞书", windowsterminal: "终端", cmd: "命令提示符", powershell: "PowerShell",
  pwsh: "PowerShell", wps: "WPS", et: "WPS 表格", wpp: "WPS 演示", snippingtool: "截图工具",
  screenclippinghost: "截图工具", mspaint: "画图", obsidian: "Obsidian", notion: "Notion",
  devenv: "Visual Studio", idea64: "IntelliJ IDEA", pycharm64: "PyCharm", typora: "Typora",
};
function appName(s) {
  if (!s) return "";
  return APP_NAMES[s.toLowerCase()] || s;
}
function hue(s) {
  let h = 0;
  for (const c of s) h = (h * 31 + c.codePointAt(0)) % 360;
  return h;
}

function toast(msg, ms = 1800) {
  const t = $("#toast");
  t.textContent = msg;
  t.classList.add("show");
  clearTimeout(toast._t);
  toast._t = setTimeout(() => t.classList.remove("show"), ms);
}

// ================= 过滤与搜索 =================
function matches(it, terms) {
  if (!terms.length) return true;
  const hay = (it.text + " " + appName(it.source)).toLowerCase();
  return terms.every((t) => hay.includes(t));
}

function passFilter(it) {
  const k = classify(it);
  switch (state.filter) {
    case "all": return true;
    case "pinned": return it.pinned;
    case "text": return k === "text" || k === "num" || k === "mail" || k === "color";
    default: return k === state.filter;
  }
}

function computeView() {
  const terms = q.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const ok = state.items.filter((it) => passFilter(it) && matches(it, terms));
  const pinned = ok.filter((i) => i.pinned);
  const rest = ok.filter((i) => !i.pinned);
  state.view = state.filter === "pinned" ? pinned : [...pinned, ...rest];
  state.pinnedCount = state.filter === "pinned" ? 0 : pinned.length;
  state.terms = terms;
}

function highlight(text, terms) {
  // 匹配位置较靠后时，从匹配处附近开始截取，保证高亮可见
  let start = 0;
  if (terms.length) {
    const i = text.toLowerCase().indexOf(terms[0]);
    if (i > 60) start = i - 20;
  }
  let s = text.slice(start, start + 400);
  if (start > 0) s = "…" + s.replace(/^\S*\s?/, "");
  let html = esc(s);
  for (const t of terms) {
    const re = new RegExp(esc(t).replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi");
    html = html.replace(re, (m) => `<mark>${m}</mark>`);
  }
  return html;
}

// ================= 搜索框状态 =================
// 面板打开时搜索框保持焦点（中文输入法需要），但为空时隐藏光标和高亮
let searchClicked = false, composing = false;
function syncSearch() {
  $(".search").classList.toggle("active", !!q.value || composing || searchClicked);
}
$(".search").addEventListener("mousedown", () => { searchClicked = true; syncSearch(); });
q.addEventListener("compositionstart", () => { composing = true; syncSearch(); });
q.addEventListener("compositionend", () => { composing = false; syncSearch(); });
q.addEventListener("blur", () => { searchClicked = false; syncSearch(); });

// ================= 渲染 =================
function itemHTML(it, idx, opts) {
  const k = classify(it);
  const K = KIND[k];
  let badge, content;
  if (k === "color") {
    badge = `<div class="badge swatch" style="--swatch:${esc(it.text.trim())}"></div>`;
  } else {
    badge = `<div class="badge" style="--k:${K.color}">${svg(K.icon)}</div>`;
  }
  if (k === "image" && it.img) {
    // 按原图比例预先算出显示尺寸（最大 200×64），卡片高度在图片加载前就是最终值
    const sc = Math.min(200 / it.img.w, 64 / it.img.h, 1);
    const tw = Math.max(1, Math.round(it.img.w * sc)), th = Math.max(1, Math.round(it.img.h * sc));
    content = `<img class="thumb" src="${IMG_BASE}${it.img.thumb}" width="${tw}" height="${th}" style="width:${tw}px;height:${th}px" draggable="false" alt="">`;
  } else if (k === "files") {
    const names = it.files.map((f) => f.split(/[\\/]/).pop());
    const label = names.length > 1 ? `${names[0]} 等 ${names.length} 个文件` : names[0] || "";
    content = `<div class="content">${highlight(label, state.terms)}</div>`;
  } else {
    const shown = k === "code" ? it.text.replace(/^\s*\n/, "") : it.text.trim();
    content = `<div class="content${k === "code" ? " mono" : ""}">${highlight(shown, state.terms)}</div>`;
  }
  const src = it.source
    ? `<span class="src"><span class="src-dot" style="--h:${hue(it.source)}">${esc(appName(it.source)[0].toUpperCase())}</span>${esc(appName(it.source))}</span><span class="dot-sep"></span>`
    : "";
  const extra = k === "image" && it.img ? `<span class="dot-sep"></span><span>${it.img.w}×${it.img.h}</span>${it.files.length ? `<span class="dot-sep"></span><span class="fname">${esc(it.files[0].split(/[\\/]/).pop())}</span>` : ""}`
    : it.len > 120 ? `<span class="dot-sep"></span><span>${it.len.toLocaleString()} 字</span>` : "";
  const cls = ["item"];
  if (it.pinned) cls.push("pinned");
  if (opts.fresh) cls.push("fresh");
  else if (opts.enter) cls.push("enter");
  const delay = opts.enter ? ` style="animation-delay:${Math.min(idx, 14) * 22}ms"` : "";
  return `<div class="${cls.join(" ")}" data-id="${it.id}" data-i="${idx}" role="option"${delay}>
    <span class="num">${idx < 9 ? idx + 1 : ""}</span>
    ${badge}
    <div class="body">${content}<div class="meta">${src}<span>${ago(it.used)}</span>${extra}</div></div>
    <span class="act pin-mark">${svg(ICON_PIN)}</span>
    <div class="actions">
      <button class="act pin" title="固定 (Ctrl+P)">${svg(ICON_PIN)}</button>
      <button class="act del" title="删除 (Del)">${svg(ICON_DEL)}</button>
    </div>
  </div>`;
}

function render({ enter = false } = {}) {
  syncSearch();
  computeView();
  const parts = [];
  const v = state.view;
  v.forEach((it, i) => {
    if (state.pinnedCount && i === 0) parts.push(`<div class="section-label">已固定</div>`);
    if (state.pinnedCount && i === state.pinnedCount && i < v.length) parts.push(`<div class="section-label">最近</div>`);
    parts.push(itemHTML(it, i, { enter, fresh: it.id === state.fresh }));
  });
  for (const el of [...list.children]) if (el !== hl) el.remove();
  list.insertAdjacentHTML("beforeend", parts.join(""));
  state.fresh = null;

  const empty = $("#empty");
  empty.hidden = v.length > 0;
  if (!v.length) {
    const FILTER_EMPTY = {
      text: ["还没有文本", "复制的文字会出现在这里"],
      link: ["还没有链接", "复制的网址会出现在这里"],
      code: ["还没有代码", "复制的代码片段会出现在这里"],
      image: ["还没有图片", "复制或截图的图片会出现在这里"],
      pinned: ["还没有固定的内容", "按 Ctrl+P 或点击图钉固定常用内容"],
    };
    let title = "还没有记录", sub = "复制任何内容，它就会出现在这里";
    if (state.terms.length) [title, sub] = ["没有找到匹配的内容", "换个关键词试试"];
    else if (FILTER_EMPTY[state.filter]) [title, sub] = FILTER_EMPTY[state.filter];
    $(".empty-title").textContent = title;
    $(".empty-sub").textContent = sub;
  }
  const unpinned = state.items.filter((i) => !i.pinned).length;
  $("#count").textContent = `${unpinned} / ${state.settings?.max_items ?? 100}`;
  state.sel = Math.max(0, Math.min(state.sel, v.length - 1));
  moveHL(true);
}

function selEl() {
  return list.querySelector(`.item[data-i="${state.sel}"]`);
}

function moveHL(instant = false) {
  list.querySelectorAll(".item.sel").forEach((e) => e.classList.remove("sel"));
  const el = selEl();
  if (!el) { hl.style.opacity = 0; schedulePreview(); return; }
  el.classList.add("sel");
  if (instant) hl.classList.add("no-anim");
  hl.style.opacity = 1;
  hl.style.transform = `translateY(${el.offsetTop}px)`;
  hl.style.height = `${el.offsetHeight}px`;
  if (instant) { void hl.offsetHeight; hl.classList.remove("no-anim"); }
  const top = el.offsetTop - 30, bottom = el.offsetTop + el.offsetHeight + 8;
  if (top < list.scrollTop) list.scrollTop = Math.max(0, top);
  else if (bottom > list.scrollTop + list.clientHeight) list.scrollTop = bottom - list.clientHeight;
  schedulePreview();
}

function select(i) {
  if (!state.view.length) return;
  state.sel = Math.max(0, Math.min(i, state.view.length - 1));
  moveHL();
}

// 兜底：图片加载完成或卡片高度变化时，重新对齐高亮块
list.addEventListener("load", (e) => {
  if (e.target.tagName !== "IMG") return;
  const el = selEl();
  if (!el) return;
  hl.style.transform = `translateY(${el.offsetTop}px)`;
  hl.style.height = `${el.offsetHeight}px`;
}, true);

function current() {
  return state.view[state.sel];
}

function moveInk() {
  const t = $(`.tab[data-f="${state.filter}"]`);
  const ink = $(".tab-ink");
  ink.style.width = `${t.offsetWidth}px`;
  ink.style.transform = `translate(${t.offsetLeft}px, ${t.offsetTop}px)`;
  document.querySelectorAll(".tab").forEach((b) => b.classList.toggle("active", b === t));
}

function setFilter(f) {
  state.filter = f;
  state.sel = 0;
  moveInk();
  render({ enter: true });
}

// ================= 数据 =================
async function loadItems() {
  state.items = await invoke("get_items");
}

async function doPaste(it, mode = "paste") {
  if (!it) return;
  const el = list.querySelector(`.item[data-id="${it.id}"]`);
  el?.classList.add("flash");
  await new Promise((r) => setTimeout(r, 110));
  try {
    await invoke("paste", { id: it.id, mode });
    resetPanel();
  } catch (e) {
    toast(String(e));
  }
}

async function removeItem(it) {
  if (!it) return;
  const el = list.querySelector(`.item[data-id="${it.id}"]`);
  if (el) {
    el.classList.add("leaving");
    hl.style.opacity = 0;
  }
  await invoke("delete_item", { id: it.id });
  setTimeout(() => {
    state.items = state.items.filter((x) => x.id !== it.id);
    render();
  }, el ? 210 : 0);
}

async function togglePin(it) {
  if (!it) return;
  const pinned = await invoke("toggle_pin", { id: it.id });
  if (pinned == null) return;
  it.pinned = pinned;
  await loadItems();
  render();
  // 选中跟随该条目
  const i = state.view.findIndex((x) => x.id === it.id);
  if (i >= 0) select(i);
  const el = selEl();
  el?.querySelector(pinned ? ".pin-mark" : ".act.pin")?.classList.add("pop");
  toast(pinned ? "已固定，不会被自动清理" : "已取消固定");
}

// ================= 面板显示 / 隐藏 =================
function onShow() {
  lastMouse = null;
  searchClicked = false;
  composing = false;
  state.hiding = false;
  document.body.classList.remove("hiding", "shown", "pv-open");
  state.previewOpen = false;
  closeCtx();
  if (state.settingsOpen) closeSettings(true);
  q.value = "";
  state.sel = 0;
  state.filter = "all";
  void document.body.offsetWidth;
  document.body.classList.add("shown");
  loadItems().then(() => {
    moveInk();
    render({ enter: true });
    list.scrollTop = 0;
  });
  q.focus();
}

function resetPanel() {
  document.body.classList.remove("shown", "hiding", "pv-open");
  state.previewOpen = false;
  state.hiding = false;
}

function hidePanel() {
  if (state.hiding || !document.body.classList.contains("shown")) {
    invoke("hide_panel");
    return;
  }
  state.hiding = true;
  closeCtx();
  document.body.classList.add("hiding");
  setTimeout(async () => {
    await invoke("hide_panel");
    resetPanel();
  }, 140);
}

// ================= 预览 =================
async function openPreview() {
  if (state.previewOpen || !current()) return;
  state.previewOpen = true;
  await invoke("set_preview", { open: true });
  fillPreview();
  requestAnimationFrame(() => document.body.classList.add("pv-open"));
}

function closePreview() {
  if (!state.previewOpen) return;
  state.previewOpen = false;
  document.body.classList.remove("pv-open");
  setTimeout(() => { if (!state.previewOpen) invoke("set_preview", { open: false }); }, 220);
}

function schedulePreview() {
  if (!state.previewOpen) return;
  clearTimeout(schedulePreview._t);
  schedulePreview._t = setTimeout(fillPreview, 50);
}

function colorInfo(c) {
  const cv = document.createElement("canvas").getContext("2d");
  cv.fillStyle = c;
  const v = cv.fillStyle; // "#rrggbb" 或 "rgba(r, g, b, a)"
  let r, g, b, a = 1;
  if (v.startsWith("#")) {
    r = parseInt(v.slice(1, 3), 16); g = parseInt(v.slice(3, 5), 16); b = parseInt(v.slice(5, 7), 16);
  } else {
    [r, g, b, a] = v.match(/[\d.]+/g).map(Number);
  }
  const hex = "#" + [r, g, b].map((x) => x.toString(16).padStart(2, "0")).join("").toUpperCase();
  const R = r / 255, G = g / 255, B = b / 255;
  const max = Math.max(R, G, B), min = Math.min(R, G, B), l = (max + min) / 2;
  let h = 0, s = 0;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    h = max === R ? (G - B) / d + (G < B ? 6 : 0) : max === G ? (B - R) / d + 2 : (R - G) / d + 4;
    h *= 60;
  }
  return {
    HEX: hex + (a < 1 ? Math.round(a * 255).toString(16).padStart(2, "0").toUpperCase() : ""),
    RGB: a < 1 ? `rgba(${r}, ${g}, ${b}, ${a})` : `rgb(${r}, ${g}, ${b})`,
    HSL: `hsl(${Math.round(h)}, ${Math.round(s * 100)}%, ${Math.round(l * 100)}%)`,
  };
}

async function fillPreview() {
  const it = current();
  const body = $("#pv-body");
  if (!it) { body.innerHTML = ""; return; }
  const full = (await invoke("get_item", { id: it.id })) || it;
  if (current()?.id !== it.id) return;
  const k = classify(it);
  const K = KIND[k];
  const chip = $("#pv-type");
  chip.textContent = K.label;
  chip.style.setProperty("--k", K.color);
  const meta = [];
  if (it.source) meta.push(appName(it.source));
  meta.push(ago(it.created === it.used ? it.created : it.used));
  body.className = "";
  if (k === "image" && it.img) {
    meta.unshift(`${it.img.w}×${it.img.h}`);
    body.classList.add("center");
    body.innerHTML = `<img src="${IMG_BASE}${it.img.file}" draggable="false" alt="">`;
  } else if (k === "color") {
    const info = colorInfo(it.text.trim());
    body.innerHTML = `<div class="big-swatch" style="background:${esc(it.text.trim())}"></div>
      <div class="color-rows">${Object.entries(info).map(([a, b]) => `<b>${a}</b><span>${b}</span>`).join("")}</div>`;
  } else if (k === "files") {
    meta.unshift(`${it.files.length} 个项目`);
    body.innerHTML = it.files.map((f) => {
      const name = f.split(/[\\/]/).pop();
      return `<div class="file-row">${svg(KIND.files.icon)}<div><div>${esc(name)}</div><small>${esc(f)}</small></div></div>`;
    }).join("");
  } else {
    meta.unshift(`${full.len.toLocaleString()} 字符`);
    if (full.truncated) meta.push("已截断");
    if (full.has_html) meta.push("含格式");
    if (k === "code") body.classList.add("mono");
    body.textContent = full.text;
  }
  $("#pv-meta").textContent = meta.join(" · ");
  $("#pv-meta").title = meta.join(" · ");
  const path = $("#pv-path");
  const showPath = k === "image" && it.files.length > 0;
  path.hidden = !showPath;
  path.textContent = showPath ? it.files[0] : "";
  path.title = path.textContent;
  body.scrollTop = 0;
}

// ================= 右键菜单 =================
function openCtx(it, x, y) {
  const m = $("#ctx");
  const k = classify(it);
  m.innerHTML = `
    <button data-a="paste">${svg(ICON_PASTE)}粘贴<kbd>⏎</kbd></button>
    ${k !== "image" ? `<button data-a="plain">${svg(ICON_TXT)}${k === "files" ? "粘贴为路径文本" : "粘贴为纯文本"}<kbd>⇧⏎</kbd></button>` : ""}
    <button data-a="copy">${svg(ICON_COPY)}仅复制<kbd>Ctrl C</kbd></button>
    <button data-a="preview">${svg(ICON_EYE)}预览<kbd>→</kbd></button>
    <hr>
    <button data-a="pin">${svg(ICON_PIN)}${it.pinned ? "取消固定" : "固定"}<kbd>Ctrl P</kbd></button>
    <button data-a="del" class="danger">${svg(ICON_DEL)}删除<kbd>Del</kbd></button>`;
  m.hidden = false;
  const r = m.getBoundingClientRect();
  m.style.left = `${Math.min(x, innerWidth - r.width - 8)}px`;
  m.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
  m.onclick = (e) => {
    const a = e.target.closest("button")?.dataset.a;
    if (!a) return;
    closeCtx();
    if (a === "paste" || a === "plain" || a === "copy") doPaste(it, a);
    else if (a === "preview") openPreview();
    else if (a === "pin") togglePin(it);
    else if (a === "del") removeItem(it);
  };
}
function closeCtx() {
  $("#ctx").hidden = true;
}

// ================= 设置 =================
const ACCENTS = {
  violet: ["#7c5cff", "#4fc3f7"], sunset: ["#ff5f6d", "#ffc371"], mint: ["#11b981", "#3bd5e8"],
  ocean: ["#2563eb", "#22d3ee"], rose: ["#e84393", "#a29bfe"], mono: ["#52525b", "#a1a1aa"],
};

const darkMQ = matchMedia("(prefers-color-scheme: dark)");
function applyLook() {
  const s = state.settings;
  if (!s) return;
  const dark = s.theme === "dark" || (s.theme === "system" && darkMQ.matches);
  root.dataset.theme = dark ? "dark" : "light";
  root.dataset.accent = s.accent;
  root.classList.toggle("reduce", !!s.reduce_motion);
  $("#q-hint").textContent = s.hotkey.replace(/\+/g, " ");
}
darkMQ.addEventListener("change", applyLook);

function fillSettings() {
  const s = state.settings;
  $("#s-max").value = s.max_items;
  $("#s-max-v").textContent = s.max_items;
  $("#s-hotkey").textContent = s.hotkey;
  document.querySelectorAll("#s-theme button").forEach((b) => b.classList.toggle("on", b.dataset.v === s.theme));
  $("#s-accent").innerHTML = Object.entries(ACCENTS)
    .map(([k, [a, b]]) => `<button data-v="${k}" class="${k === s.accent ? "on" : ""}" style="--s1:${a};--s2:${b}" title="${k}"></button>`)
    .join("");
  $("#s-motion").checked = s.reduce_motion;
  $("#s-auto").checked = s.autostart;
  $("#s-dir").textContent = state.dataDir;
  $("#s-dir").title = state.dataDir;
}

async function saveSettings(patch) {
  const next = { ...state.settings, ...patch };
  state.settings = next;
  applyLook();
  try {
    state.settings = await invoke("save_settings", { settings: next });
  } catch (e) {
    toast(String(e), 3000);
    const st = await invoke("get_state");
    state.settings = st.settings;
  }
  applyLook();
  fillSettings();
  if (patch.max_items) { await loadItems(); render(); }
}

function openSettings() {
  closeCtx();
  closePreview();
  state.settingsOpen = true;
  fillSettings();
  const s = $("#settings");
  s.classList.remove("closing");
  s.hidden = false;
}
function closeSettings(instant = false) {
  state.settingsOpen = false;
  stopRecording();
  const s = $("#settings");
  if (instant) { s.hidden = true; return; }
  s.classList.add("closing");
  setTimeout(() => { s.hidden = true; s.classList.remove("closing"); }, 190);
  q.focus();
}

let recording = false;
function stopRecording() {
  recording = false;
  const b = $("#s-hotkey");
  b.classList.remove("rec");
  if (state.settings) b.textContent = state.settings.hotkey;
}
function keyName(e) {
  const c = e.code;
  if (/^Key[A-Z]$/.test(c)) return c.slice(3);
  if (/^Digit\d$/.test(c)) return c.slice(5);
  if (/^F\d{1,2}$/.test(c)) return c;
  const map = { Space: "Space", Backquote: "`", Minus: "-", Equal: "=", BracketLeft: "[", BracketRight: "]",
    Semicolon: ";", Quote: "'", Comma: ",", Period: ".", Slash: "/", Backslash: "\\", Insert: "Insert", Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown" };
  return map[c] || null;
}
function onRecordKey(e) {
  e.preventDefault();
  e.stopPropagation();
  if (e.key === "Escape") { stopRecording(); return; }
  const mods = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  const k = keyName(e);
  $("#s-hotkey").textContent = [...mods, k || "…"].join(" + ");
  if (!k) return;
  if (!mods.length && !/^F\d/.test(k)) { toast("请至少包含一个修饰键（Ctrl / Alt / Shift）"); return; }
  const hk = [...mods, k].join("+");
  recording = false;
  $("#s-hotkey").classList.remove("rec");
  saveSettings({ hotkey: hk }).then(() => toast(`快捷键已设为 ${hk}`));
}

let clearArmed = false;
function bindSettings() {
  $("#btn-settings").onclick = openSettings;
  $("#set-back").onclick = () => closeSettings();
  const range = $("#s-max");
  range.oninput = () => ($("#s-max-v").textContent = range.value);
  range.onchange = () => saveSettings({ max_items: Number(range.value) });
  $("#s-hotkey").onclick = () => {
    recording = true;
    const b = $("#s-hotkey");
    b.classList.add("rec");
    b.textContent = "请按下组合键";
    b.focus();
  };
  $("#s-theme").onclick = (e) => { const v = e.target.dataset.v; if (v) saveSettings({ theme: v }); };
  $("#s-accent").onclick = (e) => { const v = e.target.dataset.v; if (v) saveSettings({ accent: v }); };
  $("#s-motion").onchange = (e) => saveSettings({ reduce_motion: e.target.checked });
  $("#s-auto").onchange = (e) => saveSettings({ autostart: e.target.checked });
  $("#s-open").onclick = () => invoke("open_data_dir");
  $("#s-quit").onclick = () => invoke("quit");
  $("#s-clear").onclick = async (e) => {
    const b = e.currentTarget;
    if (!clearArmed) {
      clearArmed = true;
      b.textContent = "再次点击确认";
      setTimeout(() => { clearArmed = false; b.textContent = "清空"; }, 2500);
      return;
    }
    clearArmed = false;
    b.textContent = "清空";
    await invoke("clear_history");
    await loadItems();
    render();
    toast("历史已清空，固定条目已保留");
  };
}

// ================= 暂停 =================
function applyPaused(p) {
  state.paused = p;
  $("#btn-pause").classList.toggle("on", p);
  $("#btn-pause").title = p ? "恢复记录" : "暂停记录";
  $("#paused-bar").hidden = !p;
  requestAnimationFrame(() => moveHL(true));
}

// ================= 键盘 =================
document.addEventListener("keydown", (e) => {
  if (recording) return onRecordKey(e);
  const ctrl = e.ctrlKey || e.metaKey;

  if (e.key === "Escape") {
    e.preventDefault();
    if (!$("#ctx").hidden) closeCtx();
    else if (state.settingsOpen) closeSettings();
    else if (state.previewOpen) closePreview();
    else if (q.value) { q.value = ""; state.sel = 0; render(); }
    else hidePanel();
    return;
  }
  if (state.settingsOpen) return;
  closeCtx();

  const it = current();
  switch (e.key) {
    case "ArrowDown": e.preventDefault(); select(state.sel + 1); return;
    case "ArrowUp": e.preventDefault(); select(state.sel - 1); return;
    case "PageDown": e.preventDefault(); select(state.sel + 6); return;
    case "PageUp": e.preventDefault(); select(state.sel - 6); return;
    case "Home": if (!q.value) { e.preventDefault(); select(0); } return;
    case "End": if (!q.value) { e.preventDefault(); select(state.view.length - 1); } return;
    case "Enter": e.preventDefault(); doPaste(it, e.shiftKey ? "plain" : "paste"); return;
    case "Tab": {
      e.preventDefault();
      const fs = [...document.querySelectorAll(".tab")].map((b) => b.dataset.f);
      const i = fs.indexOf(state.filter);
      setFilter(fs[(i + (e.shiftKey ? fs.length - 1 : 1)) % fs.length]);
      return;
    }
    case "ArrowRight":
      if (q.selectionStart === q.value.length && !state.previewOpen) { e.preventDefault(); openPreview(); }
      return;
    case "ArrowLeft":
      if (state.previewOpen) { e.preventDefault(); closePreview(); }
      return;
    case "Delete":
      if (q.selectionStart === q.value.length) { e.preventDefault(); removeItem(it); }
      return;
  }
  if (ctrl && e.key.toLowerCase() === "p") { e.preventDefault(); togglePin(it); return; }
  if (ctrl && e.key.toLowerCase() === "c") {
    const inputSel = document.activeElement === q && q.selectionStart !== q.selectionEnd;
    if (!inputSel && !String(getSelection())) { e.preventDefault(); doPaste(it, "copy"); }
    return;
  }
  if (ctrl && e.key.toLowerCase() === "f") { e.preventDefault(); q.focus(); q.select(); return; }
  // Ctrl+数字：直接粘贴第 N 条；不带 Ctrl 的数字正常输入搜索框
  const digit = /^[1-9]$/.test(e.key) ? Number(e.key) : /^Digit[1-9]$/.test(e.code) ? Number(e.code.slice(5)) : 0;
  if (digit && ctrl && !e.altKey) {
    const target = state.view[digit - 1];
    if (target) { e.preventDefault(); state.sel = digit - 1; moveHL(); doPaste(target, e.shiftKey ? "plain" : "paste"); }
    return;
  }
  // 其他可打印字符：确保输入进搜索框
  if (e.key.length === 1 && !ctrl && !e.altKey && document.activeElement !== q) q.focus();
});

q.addEventListener("input", () => {
  state.sel = 0;
  render();
});

// ================= 鼠标 =================
// 列表滚动时 WebView 会对静止的鼠标派发 mousemove，只在坐标真正变化时跟随悬停
let lastMouse = "";
list.addEventListener("mousemove", (e) => {
  const pos = `${e.screenX},${e.screenY}`;
  if (pos === lastMouse) return;
  const first = lastMouse === null;
  lastMouse = pos;
  if (first) return; // 面板刚弹出时鼠标还没动，不抢占键盘选中
  const el = e.target.closest(".item");
  if (!el || el.classList.contains("leaving")) return;
  const i = Number(el.dataset.i);
  if (i !== state.sel) { state.sel = i; moveHL(); }
});
list.addEventListener("click", (e) => {
  const el = e.target.closest(".item");
  if (!el) return;
  const it = state.view[Number(el.dataset.i)];
  if (e.target.closest(".act.pin")) return togglePin(it);
  if (e.target.closest(".act.del")) return removeItem(it);
  doPaste(it, e.shiftKey ? "plain" : "paste");
});
list.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  const el = e.target.closest(".item");
  if (!el) return closeCtx();
  state.sel = Number(el.dataset.i);
  moveHL();
  openCtx(state.view[state.sel], e.clientX, e.clientY);
});
document.addEventListener("contextmenu", (e) => { if (!e.target.closest("#pv-body")) e.preventDefault(); });
document.addEventListener("mousedown", (e) => { if (!e.target.closest("#ctx")) closeCtx(); });
// 点击窗口透明边缘：关闭面板
$("#stage").addEventListener("mousedown", (e) => { if (e.target.id === "stage") hidePanel(); });

// 拖动窗口：在顶栏、筛选栏、底栏、预览标题等空白处按住左键即可拖动
const DRAG_ZONES = "#top, #tabs, #foot, #paused-bar, .pv-head, .pv-foot, .set-head";
const NO_DRAG = "input, button, kbd, a, .search";
document.addEventListener("mousedown", (e) => {
  if (e.button !== 0 || e.detail > 1) return;
  if (!e.target.closest(DRAG_ZONES) || e.target.closest(NO_DRAG)) return;
  e.preventDefault();
  T.window.getCurrentWindow().startDragging().catch(() => {});
});

$("#tabs").addEventListener("click", (e) => {
  const f = e.target.closest(".tab")?.dataset.f;
  if (f) { setFilter(f); q.focus(); }
});
$("#btn-close").onclick = () => hidePanel();
$("#btn-pause").onclick = () => invoke("set_pause", { paused: !state.paused });
$("#btn-resume").onclick = () => invoke("set_pause", { paused: false });
$("#pv-paste").onclick = () => doPaste(current(), "paste");
$("#pv-copy").onclick = () => doPaste(current(), "copy");

// ================= 启动 =================
async function init() {
  const st = await invoke("get_state");
  state.settings = st.settings;
  state.dataDir = st.dataDir;
  applyLook();
  applyPaused(st.paused);
  bindSettings();
  await loadItems();
  moveInk();
  render();

  await listen("panel-show", onShow);
  await listen("panel-hide-request", () => { if (!recording) hidePanel(); });
  await listen("open-settings", () => setTimeout(openSettings, 120));
  await listen("paused-changed", (e) => {
    applyPaused(e.payload);
    toast(e.payload ? "已暂停记录" : "已恢复记录");
  });
  await listen("items-changed", async (e) => {
    await loadItems();
    const visible = document.body.classList.contains("shown") && !state.hiding;
    if (visible && e.payload?.new != null) state.fresh = e.payload.new;
    const keep = current()?.id;
    render();
    if (keep != null) {
      const i = state.view.findIndex((x) => x.id === keep);
      if (i >= 0 && !state.fresh) { state.sel = i; moveHL(true); }
    }
  });
  // 刷新相对时间
  setInterval(() => { if (document.body.classList.contains("shown")) list.querySelectorAll(".item").forEach((el) => {
    const it = state.view[Number(el.dataset.i)];
    const t = el.querySelector(".meta > span:not(.src):not(.dot-sep)");
    if (it && t) t.textContent = ago(it.used);
  }); }, 30000);

  // 首次启动时窗口可能已可见（main.rs 会在启动时弹出一次）
  if (await T.window.getCurrentWindow().isVisible()) onShow();
}

init();

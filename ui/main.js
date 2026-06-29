// Ziplark desktop frontend. Talks to the Rust engine via Tauri's invoke; all
// archive work happens in ziplark-core, identical to the CLI.

const T = window.__TAURI__ || null;
const invoke = T ? T.core.invoke : async () => { throw new Error("Ziplark must run in the desktop app"); };
const dialog = T ? T.dialog : null;
const listen = T ? T.event.listen : null;

const $ = (id) => document.getElementById(id);
const fmtBytes = (n) => {
  if (n === 0 || n == null) return "—";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0, v = n;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(v < 10 && i > 0 ? 1 : 0)} ${u[i]}`;
};

let currentArchive = null;     // { path }
let createInputs = [];         // string[]

/* ---------- chrome: tabs, toast, busy ---------- */
function switchView(name) {
  document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("active", t.dataset.view === name));
  document.querySelectorAll(".view").forEach((v) => v.classList.toggle("active", v.id === `view-${name}`));
}
document.querySelectorAll(".tab").forEach((t) => t.addEventListener("click", () => switchView(t.dataset.view)));

let toastTimer = null;
function toast(msg, kind = "") {
  const el = $("toast");
  el.textContent = msg;
  el.className = `toast ${kind}`;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.add("hidden"), 4200);
}
function busy(on, msg = "Working…") {
  $("busy-msg").textContent = msg;
  $("busy").classList.toggle("hidden", !on);
}

/* ---------- password modal ---------- */
function askPassword(message) {
  return new Promise((resolve) => {
    const modal = $("pw-modal"), input = $("pw-input");
    $("pw-modal-msg").textContent = message || "This archive is encrypted.";
    input.value = "";
    modal.classList.remove("hidden");
    input.focus();
    const done = (val) => {
      modal.classList.add("hidden");
      $("pw-ok").onclick = $("pw-cancel").onclick = input.onkeydown = null;
      resolve(val);
    };
    $("pw-ok").onclick = () => done(input.value);
    $("pw-cancel").onclick = () => done(null);
    input.onkeydown = (e) => { if (e.key === "Enter") done(input.value); if (e.key === "Escape") done(null); };
  });
}

function isPwError(err) {
  const s = String(err).toLowerCase();
  return s.includes("password") || s.includes("encrypted");
}

/* ---------- OPEN / EXTRACT ---------- */
async function openArchive(path) {
  busy(true, "Reading archive…");
  let password = null;
  try {
    let info;
    while (true) {
      try {
        info = await invoke("list_archive", { path, password });
        break;
      } catch (err) {
        if (isPwError(err)) {
          busy(false);
          password = await askPassword(String(err));
          if (password === null) return;       // user cancelled
          busy(true, "Reading archive…");
        } else {
          throw err;
        }
      }
    }
    currentArchive = { path, password };
    renderArchive(info);
  } catch (err) {
    toast(String(err), "err");
  } finally {
    busy(false);
  }
}

function renderArchive(info) {
  $("open-drop").classList.add("hidden");
  $("archive-panel").classList.remove("hidden");
  const name = info.path.split(/[\\/]/).pop();
  $("arc-name").textContent = name;
  $("arc-meta").textContent =
    `${info.format} · ${info.entries.length} entries · ${fmtBytes(info.total_size)} uncompressed` +
    (info.encrypted ? " · 🔒 encrypted" : "");
  const tbody = $("entries").querySelector("tbody");
  tbody.innerHTML = "";
  for (const e of info.entries) {
    const tr = document.createElement("tr");
    const lock = e.encrypted ? ' <span class="lock" title="encrypted">🔒</span>' : "";
    tr.innerHTML =
      `<td>${escapeHtml(e.path)}${lock}</td>` +
      `<td class="num">${e.is_dir ? "—" : fmtBytes(e.size)}</td>` +
      `<td>${e.is_dir ? "folder" : "file"}</td>`;
    tbody.appendChild(tr);
  }
}

function escapeHtml(s) {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}

function closeArchive() {
  currentArchive = null;
  $("archive-panel").classList.add("hidden");
  $("open-drop").classList.remove("hidden");
}

async function extractArchive() {
  if (!currentArchive) return;
  const dest = await dialog.open({ directory: true, multiple: false, title: "Extract to…" });
  if (!dest) return;
  busy(true, "Extracting…");
  try {
    const r = await invoke("extract_archive", {
      path: currentArchive.path, dest, password: currentArchive.password, overwrite: true,
    });
    toast(`Extracted ${r.files_written} files (${fmtBytes(r.bytes_written)}) → ${dest}`, "ok");
  } catch (err) {
    toast(String(err), "err");
  } finally {
    busy(false);
  }
}

async function testArchive() {
  if (!currentArchive) return;
  busy(true, "Verifying…");
  try {
    const r = await invoke("test_archive", { path: currentArchive.path, password: currentArchive.password });
    if (r.ok) toast(`Integrity OK — ${r.entries_tested} entries verified`, "ok");
    else toast(`FAILED — ${r.bad_entries.length} bad entries`, "err");
  } catch (err) {
    toast(String(err), "err");
  } finally {
    busy(false);
  }
}

$("btn-open").onclick = async () => {
  const sel = await dialog.open({ multiple: false, title: "Open archive" });
  if (sel) openArchive(sel);
};
$("btn-extract").onclick = extractArchive;
$("btn-test").onclick = testArchive;
$("btn-close").onclick = closeArchive;

/* ---------- CREATE ---------- */
function renderInputs() {
  const ul = $("input-list");
  ul.innerHTML = "";
  for (const [i, p] of createInputs.entries()) {
    const li = document.createElement("li");
    li.innerHTML = `<span class="path">${escapeHtml(p)}</span>`;
    const rm = document.createElement("button");
    rm.className = "rm"; rm.textContent = "✕"; rm.title = "remove";
    rm.onclick = () => { createInputs.splice(i, 1); renderInputs(); };
    li.appendChild(rm);
    ul.appendChild(li);
  }
  $("btn-create").disabled = createInputs.length === 0;
}
function addInputs(paths) {
  for (const p of paths) if (!createInputs.includes(p)) createInputs.push(p);
  renderInputs();
}

$("btn-add").onclick = async () => {
  const sel = await dialog.open({ multiple: true, title: "Add files" });
  if (sel) addInputs(Array.isArray(sel) ? sel : [sel]);
};
$("btn-clear").onclick = () => { createInputs = []; renderInputs(); };

$("btn-create").onclick = async () => {
  if (createInputs.length === 0) return;
  const fmt = $("fmt").value;
  const ext = fmt;
  const out = await dialog.save({ title: "Save archive as…", defaultPath: `archive.${ext}` });
  if (!out) return;
  busy(true, "Creating archive…");
  try {
    const r = await invoke("create_archive", {
      output: out,
      inputs: createInputs,
      format: fmt,
      level: $("level").value,
      password: $("pw-create").value || null,
    });
    const ratio = r.bytes_in ? Math.round((100 * r.bytes_out) / r.bytes_in) : 0;
    toast(`Created ${out.split(/[\\/]/).pop()} — ${r.entries_added} entries, ${fmtBytes(r.bytes_out)} (${ratio}%)`, "ok");
  } catch (err) {
    toast(String(err), "err");
  } finally {
    busy(false);
  }
};

/* ---------- drag & drop (OS file paths via Tauri) ---------- */
function setDrag(on) {
  document.querySelectorAll(".dropzone").forEach((d) => d.classList.toggle("dragover", on));
}
if (listen) {
  listen("tauri://drag-enter", () => setDrag(true));
  listen("tauri://drag-over", () => setDrag(true));
  listen("tauri://drag-leave", () => setDrag(false));
  listen("tauri://drag-drop", (e) => {
    setDrag(false);
    const paths = (e.payload && e.payload.paths) || [];
    if (paths.length === 0) return;
    const openActive = $("view-open").classList.contains("active");
    if (openActive) {
      openArchive(paths[0]);
    } else {
      addInputs(paths);
    }
  });
}

renderInputs();

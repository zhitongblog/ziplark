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

let currentArchive = null;     // { path, password }
let createInputs = [];         // string[]

/* ---------- chrome: tabs, toast ---------- */
function switchView(name) {
  document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("active", t.dataset.view === name));
  document.querySelectorAll(".view").forEach((v) => v.classList.toggle("active", v.id === `view-${name}`));
}
document.querySelectorAll(".tab").forEach((t) => t.addEventListener("click", () => switchView(t.dataset.view)));

let toastTimer = null;
function toast(msg, kind = "", action = null) {
  const el = $("toast"), btn = $("toast-action");
  $("toast-msg").textContent = msg;
  el.className = `toast ${kind}`;
  btn.classList.toggle("hidden", !action);
  if (action) {
    btn.textContent = action.label;
    btn.onclick = () => { el.classList.add("hidden"); action.run(); };
  }
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.add("hidden"), action ? 8000 : 4200);
}

/* ---------- progress + cancel ----------
   The engine reports as it goes and stops when we answer "stop", so the
   overlay is a real progress bar with a way out, not a spinner that lies. */
let busyActive = false;

function busy(on, msg = "Working…") {
  busyActive = on;
  $("busy-msg").textContent = msg;
  $("busy-detail").textContent = "";
  setBar(null);
  $("btn-cancel").disabled = false;
  $("btn-cancel").textContent = "Cancel";
  $("busy").classList.toggle("hidden", !on);
}

function setBar(fraction) {
  const fill = $("bar-fill");
  if (fraction == null) {
    fill.classList.add("indeterminate");
    fill.style.width = "";
  } else {
    fill.classList.remove("indeterminate");
    fill.style.width = `${Math.max(0, Math.min(1, fraction)) * 100}%`;
  }
}

function shortPath(p, max = 58) {
  if (p.length <= max) return p;
  return "…" + p.slice(-(max - 1));
}

if (listen) {
  T.event.listen("ziplark://progress", (e) => {
    if (!busyActive) return;
    window.__progressCount = (window.__progressCount || 0) + 1;
    const p = e.payload;
    const parts = [];
    if (p.entries_total > 0) parts.push(`${p.entries_done} / ${p.entries_total}`);
    else if (p.entries_done > 0) parts.push(`${p.entries_done} entries`);
    if (p.bytes_done > 0) parts.push(fmtBytes(p.bytes_done));
    $("busy-detail").textContent = `${shortPath(p.current_path)}${parts.length ? "  ·  " + parts.join("  ·  ") : ""}`;

    if (p.bytes_total > 0) setBar(p.bytes_done / p.bytes_total);
    else if (p.entries_total > 0) setBar(p.entries_done / p.entries_total);
    else setBar(null);
  });
}

$("btn-cancel").onclick = async () => {
  $("btn-cancel").disabled = true;
  $("btn-cancel").textContent = "Stopping…";
  try { await invoke("cancel_operation"); } catch { /* nothing to stop */ }
};

const wasCancelled = (err) => String(err).toLowerCase().includes("cancelled");

/* ---------- modals ---------- */
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

function confirmAction(title, message, okLabel = "Continue") {
  return new Promise((resolve) => {
    const modal = $("confirm-modal");
    $("confirm-title").textContent = title;
    $("confirm-msg").textContent = message;
    $("confirm-yes").textContent = okLabel;
    modal.classList.remove("hidden");
    $("confirm-yes").focus();
    const done = (val) => {
      modal.classList.add("hidden");
      $("confirm-yes").onclick = $("confirm-no").onclick = null;
      resolve(val);
    };
    $("confirm-yes").onclick = () => done(true);
    $("confirm-no").onclick = () => done(false);
  });
}

function isPwError(err) {
  const s = String(err).toLowerCase();
  return s.includes("password") || s.includes("encrypted");
}

/* ---------- OPEN / EXTRACT ---------- */
async function openArchive(path) {
  switchView("open");
  busy(true, "Reading archive…");
  let password = null;
  try {
    let info;
    for (;;) {
      try {
        info = await invoke("list_archive", { path, password });
        break;
      } catch (err) {
        if (wasCancelled(err)) return;
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
  buildTree(info.entries);
}

function closeArchive() {
  currentArchive = null;
  $("archive-panel").classList.add("hidden");
  $("open-drop").classList.remove("hidden");
  tree.nodes = [];
  tree.rows = [];
  renderRows();
}

/* ---------- entry tree ----------
   Archive entries arrive as a flat list of paths. Showing them flat means a
   disc image dumps 200 000 rows into the DOM at once; folding them into a tree
   and only rendering the visible slice keeps it instant either way. */
const ROW_H = 26;
const tree = { nodes: [], rows: [], expanded: new Set() };

function buildTree(entries) {
  const root = { name: "", path: "", dir: true, size: 0, children: new Map(), depth: -1 };

  for (const e of entries) {
    const parts = e.path.split("/").filter(Boolean);
    if (parts.length === 0) continue;
    let node = root;
    parts.forEach((part, i) => {
      const last = i === parts.length - 1;
      let child = node.children.get(part);
      if (!child) {
        child = {
          name: part,
          path: parts.slice(0, i + 1).join("/"),
          dir: last ? e.is_dir : true,
          size: 0,
          encrypted: false,
          children: new Map(),
          depth: i,
        };
        node.children.set(part, child);
      }
      if (last) {
        child.dir = e.is_dir;
        child.size = e.size;
        child.encrypted = e.encrypted;
      }
      node = child;
    });
  }

  tree.expanded = new Set();
  // Open the first level so the archive doesn't look empty on arrival.
  for (const child of root.children.values()) {
    if (child.dir) tree.expanded.add(child.path);
  }
  tree.nodes = sortChildren(root);
  flatten();
  $("tree").scrollTop = 0;
  renderRows();
}

function sortChildren(node) {
  const kids = [...node.children.values()];
  kids.sort((a, b) => (a.dir === b.dir ? a.name.localeCompare(b.name) : a.dir ? -1 : 1));
  for (const k of kids) k.sorted = sortChildren(k);
  return kids;
}

/// The rows currently visible, given which folders are open.
function flatten() {
  const rows = [];
  const walk = (nodes) => {
    for (const n of nodes) {
      rows.push(n);
      if (n.dir && tree.expanded.has(n.path) && n.sorted?.length) walk(n.sorted);
    }
  };
  walk(tree.nodes);
  tree.rows = rows;
  $("tree-sizer").style.height = `${rows.length * ROW_H}px`;
}

function escapeHtml(s) {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}

function renderRows() {
  const view = $("tree"), holder = $("tree-rows");
  const first = Math.max(0, Math.floor(view.scrollTop / ROW_H) - 6);
  const count = Math.ceil(view.clientHeight / ROW_H) + 12;
  const slice = tree.rows.slice(first, first + count);

  holder.style.transform = `translateY(${first * ROW_H}px)`;
  holder.innerHTML = slice
    .map((n) => {
      const open = tree.expanded.has(n.path);
      const twisty = n.dir && n.sorted?.length
        ? `<span class="twisty${open ? " open" : ""}">▸</span>`
        : `<span class="twisty empty"></span>`;
      const icon = n.dir ? "📁" : "📄";
      const lock = n.encrypted ? ' <span class="lock" title="encrypted">🔒</span>' : "";
      return (
        `<div class="row" data-path="${escapeHtml(n.path)}" data-dir="${n.dir}" style="padding-left:${8 + n.depth * 16}px">` +
        `${twisty}<span class="ricon">${icon}</span>` +
        `<span class="rname">${escapeHtml(n.name)}${lock}</span>` +
        `<span class="rsize">${n.dir ? "" : fmtBytes(n.size)}</span>` +
        `</div>`
      );
    })
    .join("");
}

$("tree").addEventListener("scroll", renderRows);
$("tree").addEventListener("click", (e) => {
  const row = e.target.closest(".row");
  if (!row || row.dataset.dir !== "true") return;
  const path = row.dataset.path;
  if (tree.expanded.has(path)) tree.expanded.delete(path);
  else tree.expanded.add(path);
  flatten();
  renderRows();
});
window.addEventListener("resize", renderRows);

async function extractArchive() {
  if (!currentArchive) return;
  const dest = await dialog.open({ directory: true, multiple: false, title: "Extract to…" });
  if (!dest) return;
  await runExtract(dest, false);
}

async function runExtract(dest, overwrite) {
  busy(true, "Extracting…");
  try {
    const r = await invoke("extract_archive", {
      path: currentArchive.path,
      dest,
      password: currentArchive.password,
      overwrite,
      include: null,
    });
    toast(
      `Extracted ${r.files_written} files (${fmtBytes(r.bytes_written)}) → ${dest}`,
      "ok",
      { label: "Show", run: () => invoke("reveal_in_file_manager", { path: dest }).catch(() => {}) }
    );
  } catch (err) {
    if (wasCancelled(err)) {
      toast("Extraction stopped. Files written so far were kept.", "");
      return;
    }
    // The engine refuses to clobber unless told to; ask rather than decide.
    if (!overwrite && String(err).includes("already exists")) {
      busy(false);
      const ok = await confirmAction(
        "Some files already exist",
        `${String(err).replace(/ \(use overwrite\)$/, "")}\n\nReplace existing files in this folder?`,
        "Replace"
      );
      if (ok) await runExtract(dest, true);
      return;
    }
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
    if (wasCancelled(err)) toast("Verification stopped.", "");
    else toast(String(err), "err");
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

// A mistyped password on a new archive is unrecoverable, so it has to be typed
// twice and the mismatch has to be visible before the archive is written.
function passwordsMatch() {
  const a = $("pw-create").value, b = $("pw-create2").value;
  const ok = a === b;
  $("pw-mismatch").classList.toggle("hidden", ok || (!a && !b));
  return ok;
}
$("pw-create").oninput = $("pw-create2").oninput = passwordsMatch;

$("btn-create").onclick = async () => {
  if (createInputs.length === 0) return;
  if (!passwordsMatch()) {
    toast("The two passwords don't match.", "err");
    $("pw-create2").focus();
    return;
  }
  const fmt = $("fmt").value;
  const out = await dialog.save({ title: "Save archive as…", defaultPath: `archive.${fmt}` });
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
    toast(
      `Created ${out.split(/[\\/]/).pop()} — ${r.entries_added} entries, ${fmtBytes(r.bytes_out)} (${ratio}%)`,
      "ok",
      { label: "Show", run: () => invoke("reveal_in_file_manager", { path: out }).catch(() => {}) }
    );
  } catch (err) {
    if (wasCancelled(err)) toast("Archive creation stopped. The partial file was left in place.", "");
    else toast(String(err), "err");
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

  // "Open with Ziplark" / dropping an archive on the dock icon.
  listen("ziplark-open-file", (e) => { if (e.payload) openArchive(e.payload); });
}

renderInputs();

/* ---------- startup ---------- */
if (T) {
  invoke("app_version")
    .then((v) => { $("app-ver").textContent = "Ziplark v" + v; })
    .catch(() => {});
  // An archive the OS handed us before the window was listening.
  invoke("take_pending_file")
    .then(async (p) => {
      if (p) await openArchive(p);
      // Debug-only hook so the window can be driven without synthetic clicks.
      const action = await invoke("selftest_action").catch(() => null);
      if (action === "verify") {
        await testArchive();
        await invoke("selftest_log", {
          line: `verify finished; progress events seen by the window = ${window.__progressCount || 0}`,
        });
      }
      if (action === "verify-cancel") {
        const done = testArchive();
        setTimeout(() => $("btn-cancel").click(), 1500);
        await done;
        await invoke("selftest_log", {
          line: `verify-cancel finished; events=${window.__progressCount || 0}; toast="${$("toast-msg").textContent}"`,
        });
      }
      if (action === "extract-overwrite") {
        const dest = p.replace(/[^/\\]+$/, "selftest-out");
        await runExtract(dest, false);
        const second = runExtract(dest, false);
        await new Promise((r) => setTimeout(r, 900));
        const asked = !$("confirm-modal").classList.contains("hidden");
        if (asked) $("confirm-yes").click();
        await second;
        await invoke("selftest_log", {
          line: `extract-overwrite finished; asked-before-replacing=${asked}; toast="${$("toast-msg").textContent}"`,
        });
      }
      if (action === "extract-cancel") {
        const dest = p.replace(/[^/\\]+$/, "selftest-out");
        const done = runExtract(dest, true);
        setTimeout(() => $("btn-cancel").click(), 1500);
        await done;
        await invoke("selftest_log", {
          line: `extract-cancel finished; events=${window.__progressCount || 0}; toast="${$("toast-msg").textContent}"`,
        });
      }
    })
    .catch((err) => {
      invoke("selftest_log", { line: `hook: FAILED ${err}` }).catch(() => {});
    });
}
document.querySelectorAll(".appfoot .ext").forEach((a) => {
  a.addEventListener("click", (e) => {
    e.preventDefault();
    if (T) invoke("open_url", { url: a.dataset.url }).catch(() => {});
  });
});


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

  // Facts about the archive as a whole. Most formats have none of these; a RAR
  // set can have all of them, and "3 volumes" or "solid" changes what the user
  // should expect.
  const bits = [
    info.format,
    `${info.entries.length} entries`,
    `${fmtBytes(info.total_size)} uncompressed`,
  ];
  if (info.volumes?.length > 1) bits.push(`${info.volumes.length} volumes`);
  if (info.encrypted) bits.push("🔒 encrypted");
  const a = info.attributes || {};
  if (a.solid) bits.push("solid");
  if (a.recovery_record) bits.push("recovery record");
  if (a.locked) bits.push("locked");
  $("arc-meta").textContent = bits.join(" · ");

  const notice = $("arc-notice");
  if (info.missing_volume) {
    // The listing is real but partial: say so, and offer the one action that
    // actually helps rather than letting Extract fail later.
    const missing = info.missing_volume.split(/[\\/]/).pop();
    notice.textContent = `Incomplete: this set continues into ${missing}, which isn't in the folder.`;
    const salvage = document.createElement("button");
    salvage.className = "ghost";
    salvage.textContent = "Extract what's readable…";
    salvage.onclick = () => extractArchive({ keepBroken: true });
    notice.appendChild(salvage);
    notice.classList.remove("hidden");
  } else {
    notice.textContent = "";
    notice.classList.add("hidden");
  }

  const comment = $("arc-comment");
  if (info.comment) {
    comment.textContent = info.comment;
    comment.classList.remove("hidden");
  } else {
    comment.textContent = "";
    comment.classList.add("hidden");
  }

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
const tree = { nodes: [], rows: [], expanded: new Set(), selected: new Set() };

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
  tree.selected = new Set();
  updateSelectionUi();
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

function updateSelectionUi() {
  const btn = $("btn-extract-sel");
  const n = tree.selected.size;
  btn.classList.toggle("hidden", n === 0);
  btn.textContent = n === 1 ? "Extract 1 selected…" : `Extract ${n} selected…`;
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
      const checked = tree.selected.has(n.path) ? " on" : "";
      return (
        `<div class="row" data-path="${escapeHtml(n.path)}" data-dir="${n.dir}" style="padding-left:${8 + n.depth * 16}px">` +
        `<span class="rcheck${checked}" title="select"></span>` +
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
  if (!row) return;
  const path = row.dataset.path;

  // Ticking a row selects it for extraction. Ticking a folder takes everything
  // under it, which is what the engine's exact matching already means by a
  // directory path — so there is no subtree bookkeeping to get wrong here.
  if (e.target.closest(".rcheck")) {
    if (tree.selected.has(path)) tree.selected.delete(path);
    else tree.selected.add(path);
    updateSelectionUi();
    renderRows();
    return;
  }

  if (row.dataset.dir !== "true") return;
  if (tree.expanded.has(path)) tree.expanded.delete(path);
  else tree.expanded.add(path);
  flatten();
  renderRows();
});
window.addEventListener("resize", renderRows);

async function extractArchive(opts = {}) {
  if (!currentArchive) return;
  const dest = await dialog.open({ directory: true, multiple: false, title: "Extract to…" });
  if (!dest) return;
  await runExtract(dest, { overwrite: false, ...opts });
}

/// Extract only the rows the user ticked. Their paths are exactly what the
/// listing reported, so they are sent as exact paths rather than patterns —
/// `docs/a.txt` must not also drag in `backup/docs/a.txt.bak`.
async function extractSelected() {
  if (!currentArchive || tree.selected.size === 0) return;
  const dest = await dialog.open({ directory: true, multiple: false, title: "Extract selected to…" });
  if (!dest) return;
  await runExtract(dest, { overwrite: false, include: [...tree.selected], exact: true });
}

async function runExtract(dest, opts = {}) {
  const { overwrite = false, include = null, exact = false, keepBroken = false } = opts;
  busy(true, keepBroken ? "Extracting what's readable…" : "Extracting…");
  try {
    const r = await invoke("extract_archive", {
      path: currentArchive.path,
      dest,
      password: currentArchive.password,
      overwrite,
      include,
      exact,
      keepBroken,
    });
    const failed = r.failed?.length ?? 0;
    const show = {
      label: "Show",
      run: () => invoke("reveal_in_file_manager", { path: dest }).catch(() => {}),
    };
    // A partial result has to read as partial; the file count alone would look
    // like a clean run.
    if (failed > 0) {
      const partial = r.partial?.length ?? 0;
      toast(
        `Recovered ${r.files_written} ${r.files_written === 1 ? "file" : "files"} ` +
          `(${fmtBytes(r.bytes_written)}) → ${dest}. ` +
          `${failed} could not be extracted` +
          (partial > 0 ? `; ${partial} left incomplete (${r.partial.join(", ")})` : "") +
          `: ${r.failed[0]}`,
        "",
        show
      );
    } else {
      toast(
        `Extracted ${r.files_written} ${r.files_written === 1 ? "file" : "files"} ` +
          `(${fmtBytes(r.bytes_written)}) → ${dest}`,
        "ok",
        show
      );
    }
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
      if (ok) await runExtract(dest, { ...opts, overwrite: true });
      return;
    }
    // Damage and a missing volume are both recoverable-in-part, and the user
    // cannot know that unless offered: RAR sets arrive short a volume, and
    // downloads arrive bit-rotted.
    if (!keepBroken && isSalvageable(err)) {
      busy(false);
      const ok = await confirmAction(
        "This archive is damaged or incomplete",
        `${String(err)}\n\nExtract the files that are readable and list the rest?`,
        "Extract what's readable"
      );
      if (ok) await runExtract(dest, { ...opts, keepBroken: true });
      return;
    }
    toast(String(err), "err");
  } finally {
    busy(false);
  }
}

function isSalvageable(err) {
  const s = String(err).toLowerCase();
  return (
    s.includes("corrupt") ||
    s.includes("damaged") ||
    s.includes("checksum") ||
    s.includes("truncated") ||
    s.includes("incomplete") ||
    s.includes("missing")
  );
}

async function testArchive() {
  if (!currentArchive) return;
  busy(true, "Verifying…");
  try {
    const r = await invoke("test_archive", { path: currentArchive.path, password: currentArchive.password });
    if (r.ok) {
      toast(`Integrity OK — ${r.entries_tested} entries verified`, "ok");
    } else {
      // Naming the first bad entry is the difference between "it's broken" and
      // knowing which file to re-download.
      toast(
        `${r.bad_entries.length} bad ${r.bad_entries.length === 1 ? "entry" : "entries"}, ` +
          `${r.entries_tested} verified — ${r.bad_entries[0]}`,
        "err",
        { label: "Extract readable…", run: () => extractArchive({ keepBroken: true }) }
      );
    }
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
$("btn-extract").onclick = () => extractArchive();
$("btn-extract-sel").onclick = extractSelected;
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
        await runExtract(dest, { overwrite: false });
        const second = runExtract(dest, { overwrite: false });
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
        const done = runExtract(dest, { overwrite: true });
        setTimeout(() => $("btn-cancel").click(), 1500);
        await done;
        await invoke("selftest_log", {
          line: `extract-cancel finished; events=${window.__progressCount || 0}; toast="${$("toast-msg").textContent}"`,
        });
      }
      // What the window shows for a multi-volume RAR set, and whether the
      // "extract what's readable" route out of an incomplete one works.
      if (action === "rar-volumes") {
        await invoke("selftest_log", {
          line: `rar-volumes; name="${$("arc-name").textContent}"; meta="${$("arc-meta").textContent}"` +
            `; notice-hidden=${$("arc-notice").classList.contains("hidden")}` +
            `; comment="${$("arc-comment").textContent.replace(/\n/g, "\\n")}"` +
            `; rows=${tree.rows.length}`,
        });
      }
      if (action === "rar-salvage") {
        const shown = !$("arc-notice").classList.contains("hidden");
        const dest = p.replace(/[^/\\]+$/, "selftest-salvage");
        await runExtract(dest, { overwrite: true, keepBroken: true });
        await invoke("selftest_log", {
          line: `rar-salvage; notice-shown=${shown}; notice="${$("arc-notice").textContent}"` +
            `; toast="${$("toast-msg").textContent}"`,
        });
      }
      // Ticking rows and extracting only those, which is what the exact
      // matcher exists for.
      if (action === "extract-selected") {
        const files = tree.rows.filter((n) => !n.dir).slice(0, 1);
        files.forEach((n) => tree.selected.add(n.path));
        updateSelectionUi();
        renderRows();
        const dest = p.replace(/[^/\\]+$/, "selftest-selected");
        await runExtract(dest, { overwrite: true, include: [...tree.selected], exact: true });
        await invoke("selftest_log", {
          line: `extract-selected; picked=${JSON.stringify([...tree.selected])}` +
            `; button="${$("btn-extract-sel").textContent}"` +
            `; button-hidden=${$("btn-extract-sel").classList.contains("hidden")}` +
            `; toast="${$("toast-msg").textContent}"`,
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


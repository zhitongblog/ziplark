// Ziplark desktop frontend. Talks to the Rust engine via Tauri's invoke; all
// archive work happens in ziplark-core, identical to the CLI.

const T = window.__TAURI__ || null;
const invoke = T ? T.core.invoke : async () => { throw new Error("Ziplark must run in the desktop app"); };
const dialog = T ? T.dialog : null;
const listen = T ? T.event.listen : null;

// Surface uncaught errors in the dev log (`selftest_log` is a no-op in release
// builds), so a broken handler shows up as a line instead of a dead button.
if (T) {
  const report = (line) => invoke("selftest_log", { line }).catch(() => {});
  window.addEventListener("error", (e) =>
    report(`js-error: ${e.message} @ ${e.filename}:${e.lineno}:${e.colno}`));
  window.addEventListener("unhandledrejection", (e) =>
    report(`js-rejection: ${e.reason && (e.reason.stack || e.reason)}`));
  // TEMP diagnostics: where do real pointer events land?
  const desc = (el) => el ? `${el.tagName}#${el.id || ""}.${(el.className || "").toString().replace(/\s+/g, ".")}` : "null";
  for (const type of ["pointerdown", "mousedown", "click"]) {
    document.addEventListener(type, (e) =>
      report(`ev ${type} target=${desc(e.target)} at=${e.clientX},${e.clientY} top=${desc(document.elementFromPoint(e.clientX, e.clientY))}`), true);
  }
  window.addEventListener("blur", () => report("ev window-blur"));
  window.addEventListener("focus", () => report("ev window-focus"));
}

const $ = (id) => document.getElementById(id);
const fmtBytes = (n) => {
  if (n === 0 || n == null) return "—";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0, v = n;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(v < 10 && i > 0 ? 1 : 0)} ${u[i]}`;
};

const fmtDate = (sec) => {
  if (!sec || sec <= 0) return "";
  try {
    const d = new Date(sec * 1000);
    const pad = (x) => String(x).padStart(2, "0");
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  } catch {
    return "";
  }
};

const MODIFIABLE_FORMATS = new Set(["zip", "sevenz", "7z", "tar", "targz", "tarbz2", "tarxz", "tarzst", "tarlz4"]);

let currentArchive = null;     // { path, password, format }
let createInputs = [];         // string[]

/* ---------- i18n (9 languages) ---------- */
const I18N = {
  en: {
    tagline: "Free · fast · every archive",
    tab_open: "Open",
    tab_create: "Create",
    btn_context_menu: "Right-click menu",
    drop_archive: "Drop an archive here",
    or: "or",
    choose_archive: "Choose archive…",
    btn_verify: "Verify",
    btn_add_files: "Add files…",
    btn_extract_here: "Extract here",
    btn_extract_all: "Extract all…",
    btn_close: "Close",
    search_placeholder: "Search or filter entries (e.g. *.txt, docs/)…",
    dblclick_hint: "Double-click a file to preview",
    col_name: "Name",
    col_modified: "Modified",
    col_size: "Size",
    drop_files: "Drop files & folders here",
    add_btn: "add…",
    lbl_format: "Format",
    lbl_compression: "Compression",
    lvl_default: "Default",
    lvl_best: "Best (smallest)",
    lvl_fast: "Fast",
    lvl_store: "Store (none)",
    lbl_password: "Password",
    lbl_repeat_password: "Repeat password",
    pw_optional: "leave empty for none",
    pw_repeat: "type it again",
    pw_mismatch: "The two passwords don't match.",
    btn_clear: "Clear",
    btn_create_archive: "Create archive…",
    pw_required: "Password required",
    btn_cancel: "Cancel",
    btn_unlock: "Unlock",
    btn_copy: "Copy",
    btn_extract_this: "Extract file…",
    shell_title: "Right-Click Context Menu",
    shell_desc: 'Add "Extract here with Ziplark" and "Compress to ZIP with Ziplark" to your OS file manager.',
    btn_remove: "Remove",
    btn_install: "Enable",
  },
  "zh-CN": {
    tagline: "免费 · 极速 · 全格式归档工具",
    tab_open: "打开归档",
    tab_create: "创建归档",
    btn_context_menu: "右键菜单",
    drop_archive: "将压缩包拖放到此处",
    or: "或",
    choose_archive: "选择归档文件…",
    btn_verify: "校验",
    btn_add_files: "添加文件…",
    btn_extract_here: "智能解压到当前目录",
    btn_extract_all: "全部解压…",
    btn_close: "关闭",
    search_placeholder: "搜索或过滤条目（例如 *.txt, docs/）…",
    dblclick_hint: "双击文件可直接预览",
    col_name: "名称",
    col_modified: "修改时间",
    col_size: "大小",
    drop_files: "将文件或文件夹拖放到此处",
    add_btn: "添加…",
    lbl_format: "格式",
    lbl_compression: "压缩等级",
    lvl_default: "标准",
    lvl_best: "最高压缩 (最小体积)",
    lvl_fast: "快速",
    lvl_store: "仅存储 (不压缩)",
    lbl_password: "密码",
    lbl_repeat_password: "确认密码",
    pw_optional: "留空则不加密",
    pw_repeat: "再次输入密码",
    pw_mismatch: "两次输入的密码不一致。",
    btn_clear: "清空",
    btn_create_archive: "创建压缩包…",
    pw_required: "需要密码",
    btn_cancel: "取消",
    btn_unlock: "解锁",
    btn_copy: "复制内容",
    btn_extract_this: "单独解压此文件…",
    shell_title: "系统右键菜单集成",
    shell_desc: "在系统文件管理器右键菜单中添加“使用 Ziplark 解压到此处”与“使用 Ziplark 压缩为 ZIP”。",
    btn_remove: "移除",
    btn_install: "启用右键菜单",
  },
  "zh-TW": {
    tagline: "免費 · 極速 · 全格式封存工具",
    tab_open: "開啟封存檔",
    tab_create: "建立封存檔",
    btn_context_menu: "右鍵選單",
    drop_archive: "將壓縮檔拖放到此處",
    or: "或",
    choose_archive: "選擇壓縮檔…",
    btn_verify: "驗證",
    btn_add_files: "加入檔案…",
    btn_extract_here: "智慧解壓到此處",
    btn_extract_all: "全部解壓…",
    btn_close: "關閉",
    search_placeholder: "搜尋或過濾項目（例如 *.txt, docs/）…",
    dblclick_hint: "連按兩下檔案即可預覽",
    col_name: "名稱",
    col_modified: "修改時間",
    col_size: "大小",
    drop_files: "將檔案或資料夾拖放到此處",
    add_btn: "新增…",
    lbl_format: "格式",
    lbl_compression: "壓縮等級",
    lvl_default: "預設",
    lvl_best: "最高壓縮",
    lvl_fast: "快速",
    lvl_store: "僅儲存",
    lbl_password: "密碼",
    lbl_repeat_password: "確認密碼",
    pw_optional: "留空則不加密",
    pw_repeat: "再次輸入密碼",
    pw_mismatch: "兩次輸入的密碼不一致。",
    btn_clear: "清除",
    btn_create_archive: "建立壓縮檔…",
    pw_required: "需要密碼",
    btn_cancel: "取消",
    btn_unlock: "解鎖",
    btn_copy: "複製",
    btn_extract_this: "解壓此檔案…",
    shell_title: "系統右鍵選單整合",
    shell_desc: "在系統檔案管理員右鍵選單中加入「使用 Ziplark 解壓到此處」與「壓縮為 ZIP」。",
    btn_remove: "移除",
    btn_install: "啟用",
  },
  ja: {
    tagline: "無料・高速・あらゆるアーカイブに対応",
    tab_open: "開く",
    tab_create: "作成",
    btn_context_menu: "右クリックメニュー",
    drop_archive: "アーカイブをここにドロップ",
    or: "または",
    choose_archive: "アーカイブを選択…",
    btn_verify: "検証",
    btn_add_files: "ファイル追加…",
    btn_extract_here: "ここに展開",
    btn_extract_all: "すべて展開…",
    btn_close: "閉じる",
    search_placeholder: "エントリを検索・フィルタ (例: *.txt)…",
    dblclick_hint: "ダブルクリックでプレビュー",
    col_name: "名前",
    col_modified: "更新日時",
    col_size: "サイズ",
    drop_files: "ファイルやフォルダをドロップ",
    add_btn: "追加…",
    lbl_format: "形式",
    lbl_compression: "圧縮レベル",
    lvl_default: "標準",
    lvl_best: "最高圧縮",
    lvl_fast: "高速",
    lvl_store: "無圧縮",
    lbl_password: "パスワード",
    lbl_repeat_password: "パスワード確認",
    pw_optional: "なしの場合は空欄",
    pw_repeat: "もう一度入力",
    pw_mismatch: "パスワードが一致しません。",
    btn_clear: "クリア",
    btn_create_archive: "アーカイブを作成…",
    pw_required: "パスワードが必要です",
    btn_cancel: "キャンセル",
    btn_unlock: "ロック解除",
    btn_copy: "コピー",
    btn_extract_this: "このファイルを展開…",
    shell_title: "右クリックメニュー連携",
    shell_desc: "OSのファイルマネージャーにZiplarkの展開・圧縮メニューを追加します。",
    btn_remove: "削除",
    btn_install: "有効化",
  },
  ko: {
    tagline: "무료 · 초고속 · 모든 압축 포맷 지원",
    tab_open: "열기",
    tab_create: "만들기",
    btn_context_menu: "우클릭 메뉴",
    drop_archive: "압축 파일을 여기에 드롭하세요",
    or: "또는",
    choose_archive: "압축 파일 선택…",
    btn_verify: "검사",
    btn_add_files: "파일 추가…",
    btn_extract_here: "여기에 풀기",
    btn_extract_all: "전체 압축 풀기…",
    btn_close: "닫기",
    search_placeholder: "항목 검색 또는 필터 (예: *.txt)…",
    dblclick_hint: "더블 클릭하여 파일 미리보기",
    col_name: "이름",
    col_modified: "수정일",
    col_size: "크기",
    drop_files: "파일 및 폴더를 여기에 드롭",
    add_btn: "추가…",
    lbl_format: "포맷",
    lbl_compression: "압축률",
    lvl_default: "기본",
    lvl_best: "최고 압축",
    lvl_fast: "빠름",
    lvl_store: "압축 안 함",
    lbl_password: "비밀번호",
    lbl_repeat_password: "비밀번호 확인",
    pw_optional: "비워두면 암호 없음",
    pw_repeat: "다시 입력",
    pw_mismatch: "비밀번호가 일치하지 않습니다.",
    btn_clear: "지우기",
    btn_create_archive: "압축 파일 만들기…",
    pw_required: "비밀번호 필요",
    btn_cancel: "취소",
    btn_unlock: "잠금 해제",
    btn_copy: "복사",
    btn_extract_this: "이 파일만 풀기…",
    shell_title: "우클릭 컨텍스트 메뉴",
    shell_desc: "운영체제 파일 관리자에 Ziplark 압축 풀기 및 압축 메뉴를 추가합니다.",
    btn_remove: "제거",
    btn_install: "활성화",
  },
  es: {
    tagline: "Gratis · rápido · todos los archivos",
    tab_open: "Abrir",
    tab_create: "Crear",
    btn_context_menu: "Menú contextual",
    drop_archive: "Suelta un archivo aquí",
    or: "o",
    choose_archive: "Elegir archivo…",
    btn_verify: "Verificar",
    btn_add_files: "Añadir…",
    btn_extract_here: "Extraer aquí",
    btn_extract_all: "Extraer todo…",
    btn_close: "Cerrar",
    search_placeholder: "Buscar o filtrar entradas (ej. *.txt)…",
    dblclick_hint: "Doble clic para vista previa",
    col_name: "Nombre",
    col_modified: "Modificado",
    col_size: "Tamaño",
    drop_files: "Suelta archivos y carpetas aquí",
    add_btn: "añadir…",
    btn_clear: "Limpiar",
    btn_create_archive: "Crear archivo…",
    btn_cancel: "Cancelar",
    btn_copy: "Copiar",
    btn_extract_this: "Extraer archivo…",
    shell_title: "Menú contextual del sistema",
    btn_remove: "Quitar",
    btn_install: "Activar",
  },
  fr: {
    tagline: "Gratuit · rapide · toutes les archives",
    tab_open: "Ouvrir",
    tab_create: "Créer",
    btn_context_menu: "Menu contextuel",
    drop_archive: "Déposez une archive ici",
    or: "ou",
    choose_archive: "Choisir une archive…",
    btn_verify: "Vérifier",
    btn_add_files: "Ajouter…",
    btn_extract_here: "Extraire ici",
    btn_extract_all: "Tout extraire…",
    btn_close: "Fermer",
    search_placeholder: "Rechercher ou filtrer (ex. *.txt)…",
    dblclick_hint: "Double-cliquez pour prévisualiser",
    col_name: "Nom",
    col_modified: "Modifié",
    col_size: "Taille",
    drop_files: "Déposez fichiers et dossiers ici",
    add_btn: "ajouter…",
    btn_clear: "Effacer",
    btn_create_archive: "Créer l'archive…",
    btn_cancel: "Annuler",
    btn_copy: "Copier",
    btn_extract_this: "Extraire ce fichier…",
    shell_title: "Menu clic droit du système",
    btn_remove: "Retirer",
    btn_install: "Activer",
  },
  de: {
    tagline: "Kostenlos · schnell · jedes Archiv",
    tab_open: "Öffnen",
    tab_create: "Erstellen",
    btn_context_menu: "Kontextmenü",
    drop_archive: "Archiv hier ablegen",
    or: "oder",
    choose_archive: "Archiv wählen…",
    btn_verify: "Prüfen",
    btn_add_files: "Hinzufügen…",
    btn_extract_here: "Hier entpacken",
    btn_extract_all: "Alles entpacken…",
    btn_close: "Schließen",
    search_placeholder: "Einträge suchen oder filtern (z.B. *.txt)…",
    dblclick_hint: "Doppelklick für Vorschau",
    col_name: "Name",
    col_modified: "Geändert",
    col_size: "Größe",
    drop_files: "Dateien & Ordner hier ablegen",
    add_btn: "hinzufügen…",
    btn_clear: "Leeren",
    btn_create_archive: "Archiv erstellen…",
    btn_cancel: "Abbrechen",
    btn_copy: "Kopieren",
    btn_extract_this: "Datei entpacken…",
    shell_title: "Rechtsklick-Kontextmenü",
    btn_remove: "Entfernen",
    btn_install: "Aktivieren",
  },
  ru: {
    tagline: "Бесплатный · быстрый · все форматы",
    tab_open: "Открыть",
    tab_create: "Создать",
    btn_context_menu: "Контекстное меню",
    drop_archive: "Перетащите архив сюда",
    or: "или",
    choose_archive: "Выбрать архив…",
    btn_verify: "Проверить",
    btn_add_files: "Добавить…",
    btn_extract_here: "Распаковать сюда",
    btn_extract_all: "Распаковать всё…",
    btn_close: "Закрыть",
    search_placeholder: "Поиск или фильтр (напр. *.txt)…",
    dblclick_hint: "Двойной клик для предпросмотра",
    col_name: "Имя",
    col_modified: "Изменён",
    col_size: "Размер",
    drop_files: "Перетащите файлы и папки сюда",
    add_btn: "добавить…",
    btn_clear: "Очистить",
    btn_create_archive: "Создать архив…",
    btn_cancel: "Отмена",
    btn_copy: "Копировать",
    btn_extract_this: "Извлечь файл…",
    shell_title: "Контекстное меню ОС",
    btn_remove: "Удалить",
    btn_install: "Включить",
  },
};

function applyLanguage(lang) {
  const dict = { ...I18N.en, ...(I18N[lang] || {}) };
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const key = el.dataset.i18n;
    if (dict[key]) el.textContent = dict[key];
  });
  document.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
    const key = el.dataset.i18nPlaceholder;
    if (dict[key]) el.placeholder = dict[key];
  });
  if ($("lang-select")) $("lang-select").value = lang;
  try { localStorage.setItem("ziplark-lang", lang); } catch {}
}

function initLanguage() {
  let saved = null;
  try { saved = localStorage.getItem("ziplark-lang"); } catch {}
  if (!saved) {
    const nav = (navigator.language || "en").toLowerCase();
    if (nav.startsWith("zh-tw") || nav.startsWith("zh-hk")) saved = "zh-TW";
    else if (nav.startsWith("zh")) saved = "zh-CN";
    else if (nav.startsWith("ja")) saved = "ja";
    else if (nav.startsWith("ko")) saved = "ko";
    else if (nav.startsWith("es")) saved = "es";
    else if (nav.startsWith("fr")) saved = "fr";
    else if (nav.startsWith("de")) saved = "de";
    else if (nav.startsWith("ru")) saved = "ru";
    else saved = "en";
  }
  applyLanguage(I18N[saved] ? saved : "en");
}

if ($("lang-select")) {
  $("lang-select").addEventListener("change", (e) => applyLanguage(e.target.value));
}
initLanguage();

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
    currentArchive = { path: info.path || path, password, format: info.format };
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

  // Show "Add files…" button when this archive format can be modified in-place
  // and is not a multi-volume set.
  const canModify = MODIFIABLE_FORMATS.has(String(info.format).toLowerCase()) && !(info.volumes?.length > 1);
  if ($("btn-add-to-archive")) {
    $("btn-add-to-archive").classList.toggle("hidden", !canModify);
  }

  const notice = $("arc-notice");
  if (info.missing_volume) {
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

  if ($("tree-search")) $("tree-search").value = "";
  tree.query = "";
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

/* ---------- entry tree + search + sort ---------- */
const ROW_H = 26;
const tree = {
  root: null,
  nodes: [],
  rows: [],
  expanded: new Set(),
  selected: new Set(),
  sortBy: "name",   // "name" | "mtime" | "size"
  sortAsc: true,
  query: "",
  totalFiles: 0,
};

function buildTree(entries) {
  const root = { name: "", path: "", dir: true, size: 0, mtime: 0, children: new Map(), depth: -1 };
  let totalFiles = 0;

  for (const e of entries) {
    if (!e.is_dir) totalFiles++;
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
          mtime: 0,
          encrypted: false,
          children: new Map(),
          depth: i,
        };
        node.children.set(part, child);
      }
      if (last) {
        child.dir = e.is_dir;
        child.size = e.size;
        child.mtime = e.modified || 0;
        child.encrypted = e.encrypted;
      }
      node = child;
    });
  }

  tree.root = root;
  tree.totalFiles = totalFiles;
  tree.expanded = new Set();
  tree.selected = new Set();
  updateSelectionUi();
  for (const child of root.children.values()) {
    if (child.dir) tree.expanded.add(child.path);
  }
  resortAndFlatten();
  $("tree").scrollTop = 0;
  renderRows();
}

function resortAndFlatten() {
  if (!tree.root) return;
  tree.nodes = sortChildren(tree.root);
  flatten();
  updateSortIndicators();
}

function sortChildren(node) {
  const kids = [...node.children.values()];
  const dirMul = tree.sortAsc ? 1 : -1;
  kids.sort((a, b) => {
    if (a.dir !== b.dir) return a.dir ? -1 : 1;
    if (tree.sortBy === "size") {
      if (a.size !== b.size) return (a.size - b.size) * dirMul;
    } else if (tree.sortBy === "mtime") {
      if (a.mtime !== b.mtime) return (a.mtime - b.mtime) * dirMul;
    }
    return a.name.localeCompare(b.name) * dirMul;
  });
  for (const k of kids) k.sorted = sortChildren(k);
  return kids;
}

function updateSortIndicators() {
  for (const col of ["name", "mtime", "size"]) {
    const el = $(`sort-${col}`);
    if (!el) continue;
    el.textContent = tree.sortBy === col ? (tree.sortAsc ? "▲" : "▼") : "";
  }
}

function matchesQuery(path, q) {
  if (!q) return true;
  const lp = path.toLowerCase();
  const lq = q.toLowerCase();
  if (lq.includes("*") || lq.includes("?")) {
    const reStr = "^" + lq.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".") + "$";
    try { return new RegExp(reStr, "i").test(lp); } catch { return lp.includes(lq); }
  }
  return lp.includes(lq);
}

/// The rows currently visible, given which folders are open and any active search filter.
function flatten() {
  const rows = [];
  const q = (tree.query || "").trim();
  let matchedCount = 0;

  if (!q) {
    const walk = (nodes) => {
      for (const n of nodes) {
        rows.push(n);
        if (n.dir && tree.expanded.has(n.path) && n.sorted?.length) walk(n.sorted);
      }
    };
    walk(tree.nodes);
    if ($("tree-filter-count")) $("tree-filter-count").textContent = "";
  } else {
    const filterWalk = (nodes) => {
      const kept = [];
      for (const n of nodes) {
        if (n.dir && n.sorted?.length) {
          const childRows = filterWalk(n.sorted);
          if (childRows.length > 0 || matchesQuery(n.path, q)) {
            kept.push(n, ...childRows);
          }
        } else if (matchesQuery(n.path, q)) {
          matchedCount++;
          kept.push(n);
        }
      }
      return kept;
    };
    rows.push(...filterWalk(tree.nodes));
    if ($("tree-filter-count")) {
      $("tree-filter-count").textContent = `${matchedCount} matched`;
    }
  }

  tree.rows = rows;
  $("tree-sizer").style.height = `${rows.length * ROW_H}px`;
}

if ($("tree-search")) {
  $("tree-search").addEventListener("input", (e) => {
    tree.query = e.target.value;
    flatten();
    $("tree").scrollTop = 0;
    renderRows();
  });
}

document.querySelectorAll(".tree-head .sortable").forEach((el) => {
  el.addEventListener("click", () => {
    const col = el.dataset.sort;
    if (tree.sortBy === col) tree.sortAsc = !tree.sortAsc;
    else { tree.sortBy = col; tree.sortAsc = col === "name"; }
    resortAndFlatten();
    renderRows();
  });
});

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
  const filtering = Boolean((tree.query || "").trim());

  holder.style.transform = `translateY(${first * ROW_H}px)`;
  holder.innerHTML = slice
    .map((n) => {
      const open = filtering || tree.expanded.has(n.path);
      const twisty = n.dir && n.sorted?.length
        ? `<span class="twisty${open ? " open" : ""}">▸</span>`
        : `<span class="twisty empty"></span>`;
      const icon = n.dir ? "📁" : "📄";
      const lock = n.encrypted ? ' <span class="lock" title="encrypted">🔒</span>' : "";
      const checked = tree.selected.has(n.path) ? " on" : "";
      const previewBtn = !n.dir
        ? `<button class="rpreview" title="Quick preview">👁</button>`
        : "";
      return (
        `<div class="row" data-path="${escapeHtml(n.path)}" data-dir="${n.dir}" style="padding-left:${8 + n.depth * 16}px">` +
        `<span class="rcheck${checked}" title="select"></span>` +
        `${twisty}<span class="ricon">${icon}</span>` +
        `<span class="rname">${escapeHtml(n.name)}${lock}</span>` +
        `${previewBtn}` +
        `<span class="rmtime">${n.dir ? "" : fmtDate(n.mtime)}</span>` +
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

  if (e.target.closest(".rpreview")) {
    previewEntry(path);
    return;
  }

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

$("tree").addEventListener("dblclick", (e) => {
  const row = e.target.closest(".row");
  if (!row || row.dataset.dir === "true") return;
  previewEntry(row.dataset.path);
});

window.addEventListener("resize", renderRows);

/* ---------- Quick Preview ---------- */
let currentPreviewEntry = null;

async function previewEntry(entryPath) {
  if (!currentArchive) return;
  busy(true, "Reading entry…");
  try {
    const p = await invoke("preview_archive_entry", {
      path: currentArchive.path,
      entry: entryPath,
      password: currentArchive.password,
    });
    currentPreviewEntry = entryPath;
    $("preview-title").textContent = p.path;
    const metaBits = [fmtBytes(p.size)];
    if (p.truncated) metaBits.push(`showing first ${fmtBytes(p.returned_bytes)}`);
    $("preview-meta").textContent = metaBits.join(" · ");

    $("preview-text").classList.toggle("hidden", p.kind !== "text");
    $("preview-img-wrap").classList.toggle("hidden", p.kind !== "image");
    $("preview-hex").classList.toggle("hidden", p.kind !== "binary");
    $("preview-copy").classList.toggle("hidden", p.kind !== "text");

    if (p.kind === "text") {
      $("preview-text").textContent = p.text || "";
    } else if (p.kind === "image") {
      $("preview-img").src = p.data_url || "";
    } else {
      $("preview-hex").textContent = p.hex_preview || "";
    }
    $("preview-modal").classList.remove("hidden");
  } catch (err) {
    if (!wasCancelled(err)) toast(String(err), "err");
  } finally {
    busy(false);
  }
}

if ($("preview-close")) {
  $("preview-close").onclick = () => {
    $("preview-modal").classList.add("hidden");
    $("preview-img").src = "";
  };
}
if ($("preview-copy")) {
  $("preview-copy").onclick = async () => {
    try {
      await navigator.clipboard.writeText($("preview-text").textContent);
      toast("Copied to clipboard", "ok");
    } catch {
      toast("Could not copy to clipboard", "err");
    }
  };
}
if ($("preview-extract")) {
  $("preview-extract").onclick = async () => {
    if (!currentArchive || !currentPreviewEntry) return;
    $("preview-modal").classList.add("hidden");
    const dest = await dialog.open({ directory: true, multiple: false, title: "Extract file to…" });
    if (!dest) return;
    await runExtract(dest, { overwrite: false, include: [currentPreviewEntry], exact: true });
  };
}

/* ---------- Extraction & Smart Extract Here ---------- */
async function extractArchive(opts = {}) {
  if (!currentArchive) return;
  const dest = await dialog.open({ directory: true, multiple: false, title: "Extract to…" });
  if (!dest) return;
  await runExtract(dest, { overwrite: false, ...opts });
}

async function extractArchiveHere() {
  if (!currentArchive) return;
  busy(true, "Extracting here…");
  try {
    const r = await invoke("extract_archive_here", {
      path: currentArchive.path,
      password: currentArchive.password,
      keepBroken: false,
    });
    const show = {
      label: "Show",
      run: () => invoke("reveal_in_file_manager", { path: r.dest }).catch(() => {}),
    };
    toast(
      `Extracted ${r.files_written} ${r.files_written === 1 ? "file" : "files"} (${fmtBytes(r.bytes_written)}) → ${r.dest}`,
      "ok",
      show
    );
  } catch (err) {
    if (wasCancelled(err)) {
      toast("Extraction stopped.", "");
    } else {
      toast(String(err), "err");
    }
  } finally {
    busy(false);
  }
}

async function extractSelected() {
  if (!currentArchive || tree.selected.size === 0) return;
  const dest = await dialog.open({ directory: true, multiple: false, title: "Extract selected to…" });
  if (!dest) return;
  await runExtract(dest, { overwrite: false, include: [...tree.selected], exact: true });
}

async function runExtract(dest, opts = {}) {
  const { overwrite = false, include = null, exact = false, keepBroken = false, smart = false } = opts;
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
      smart,
    });
    const failed = r.failed?.length ?? 0;
    const show = {
      label: "Show",
      run: () => invoke("reveal_in_file_manager", { path: r.dest || dest }).catch(() => {}),
    };
    if (failed > 0) {
      const partial = r.partial?.length ?? 0;
      toast(
        `Recovered ${r.files_written} ${r.files_written === 1 ? "file" : "files"} ` +
          `(${fmtBytes(r.bytes_written)}) → ${r.dest}. ` +
          `${failed} could not be extracted` +
          (partial > 0 ? `; ${partial} left incomplete (${r.partial.join(", ")})` : "") +
          `: ${r.failed[0]}`,
        "",
        show
      );
    } else {
      toast(
        `Extracted ${r.files_written} ${r.files_written === 1 ? "file" : "files"} ` +
          `(${fmtBytes(r.bytes_written)}) → ${r.dest}`,
        "ok",
        show
      );
    }
  } catch (err) {
    if (wasCancelled(err)) {
      toast("Extraction stopped. Files written so far were kept.", "");
      return;
    }
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

async function addFilesToOpenArchive(paths) {
  if (!currentArchive || !paths?.length) return;
  busy(true, "Adding files to archive…");
  try {
    const r = await invoke("add_to_archive", {
      archive: currentArchive.path,
      inputs: paths,
      password: currentArchive.password,
    });
    const info = await invoke("list_archive", {
      path: currentArchive.path,
      password: currentArchive.password,
    });
    renderArchive(info);
    toast(`Added ${r.entries_added} ${r.entries_added === 1 ? "entry" : "entries"} to archive`, "ok");
  } catch (err) {
    if (!wasCancelled(err)) toast(String(err), "err");
  } finally {
    busy(false);
  }
}

$("btn-open").onclick = async () => {
  const sel = await dialog.open({ multiple: false, title: "Open archive" });
  if (sel) openArchive(sel);
};
$("btn-extract").onclick = () => extractArchive();
if ($("btn-extract-here")) $("btn-extract-here").onclick = extractArchiveHere;
$("btn-extract-sel").onclick = extractSelected;
$("btn-test").onclick = testArchive;
$("btn-close").onclick = closeArchive;
if ($("btn-add-to-archive")) {
  $("btn-add-to-archive").onclick = async () => {
    const sel = await dialog.open({ multiple: true, title: "Add files to archive…" });
    if (sel) await addFilesToOpenArchive(Array.isArray(sel) ? sel : [sel]);
  };
}

/* ---------- OS Right-Click Menu Integration modal ---------- */
function renderShellStatus(st) {
  const box = $("shell-status-box");
  if (!box) return;
  if (!st || !st.supported) {
    box.innerHTML = `<div class="muted">Not supported on this platform.</div>`;
    return;
  }
  const rows = (st.details || [])
    .map(
      ([label, ok]) =>
        `<div class="shell-row"><span>${escapeHtml(label)}</span>` +
        `<span class="${ok ? "badge-ok" : "badge-off"}">${ok ? "✓ Installed" : "Not installed"}</span></div>`
    )
    .join("");
  box.innerHTML = `<div class="muted small" style="margin-bottom:6px">${escapeHtml(st.platform)}</div>${rows}`;
}

if ($("btn-shell-menu")) {
  $("btn-shell-menu").onclick = async () => {
    try {
      const st = await invoke("shell_integration_status");
      renderShellStatus(st);
    } catch (e) {
      $("shell-status-box").textContent = String(e);
    }
    $("shell-modal").classList.remove("hidden");
  };
}
if ($("shell-close")) {
  $("shell-close").onclick = () => $("shell-modal").classList.add("hidden");
}
if ($("shell-install")) {
  $("shell-install").onclick = async () => {
    try {
      const st = await invoke("shell_integration_install");
      renderShellStatus(st);
      toast("Right-click context menu enabled", "ok");
    } catch (e) {
      toast(String(e), "err");
    }
  };
}
if ($("shell-uninstall")) {
  $("shell-uninstall").onclick = async () => {
    try {
      const st = await invoke("shell_integration_uninstall");
      renderShellStatus(st);
      toast("Right-click context menu removed", "");
    } catch (e) {
      toast(String(e), "err");
    }
  };
}

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
  listen("tauri://drag-drop", async (e) => {
    setDrag(false);
    const paths = (e.payload && e.payload.paths) || [];
    if (paths.length === 0) return;
    const openActive = $("view-open").classList.contains("active");
    if (openActive) {
      // If an archive is already open and is modifiable, ask whether to add the
      // dropped files into it when the dropped item is not itself an archive
      // (or when multiple items are dropped).
      if (currentArchive && MODIFIABLE_FORMATS.has(String(currentArchive.format).toLowerCase())) {
        const firstFmt = paths.length === 1
          ? await invoke("detect_format", { path: paths[0] }).catch(() => null)
          : null;
        if (!firstFmt) {
          const ok = await confirmAction(
            "Add to open archive?",
            `Add ${paths.length} dropped ${paths.length === 1 ? "item" : "items"} into ${currentArchive.path.split(/[\\/]/).pop()}?`,
            "Add files"
          );
          if (ok) await addFilesToOpenArchive(paths);
          return;
        }
      }
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
      if (action === "ui-smoke") {
        const log = (l) => invoke("selftest_log", { line: `ui-smoke: ${l}` });
        const vis = (id) => { const el = $(id); return el ? !el.classList.contains("hidden") && el.offsetParent !== null : "missing"; };
        await log(`loaded; T=${!!T}; app-ver="${$("app-ver").textContent}"`);
        // What sits on top of the middle of the window? A stray overlay eats clicks.
        const top = document.elementFromPoint(innerWidth / 2, innerHeight / 2);
        await log(`elementFromPoint(center)=${top && (top.id || top.className || top.tagName)}`);
        for (const id of ["btn-open", "btn-shell-menu", "btn-extract-here", "btn-add-to-archive"]) {
          const b = $(id);
          if (!b) { await log(`${id}: MISSING`); continue; }
          const r = b.getBoundingClientRect();
          const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
          await log(`${id}: visible=${vis(id)} disabled=${b.disabled} hit=${hit && (hit.id || hit.className || hit.tagName)}`);
        }
        $("btn-shell-menu").click();
        await new Promise((r) => setTimeout(r, 800));
        await log(`after shell-menu click: shell-modal visible=${vis("shell-modal")}; text="${($("shell-modal")?.textContent || "").replace(/\s+/g, " ").slice(0, 160)}"`);
        document.querySelectorAll(".tab").forEach((t) => t.click());
        await log(`tabs clicked; view-create active=${$("view-create")?.classList.contains("active")}`);
      }
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

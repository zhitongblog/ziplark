#!/usr/bin/env node
// Generates 3 Retina macOS App Store screenshots (2880x1800, RGB no-alpha PNG)
// showing the actual Ziplark macOS desktop interface in sandboxed MAS mode.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '../..');
const OUT_DIR = path.join(ROOT, 'appstore-assets');
fs.mkdirSync(OUT_DIR, { recursive: true });

const css = fs.readFileSync(path.join(ROOT, 'ui/styles.css'), 'utf8');
const logoB64 = fs.readFileSync(path.join(ROOT, 'ui/logo.png')).toString('base64');
const CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

function frameHtml(titlebarText, innerBody) {
  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8" />
<style>
${css}
html, body {
  width: 1440px;
  height: 900px;
  margin: 0;
  padding: 0;
  overflow: hidden;
  background: #f6f7fb;
  color: #1c1f2a;
}
.mac-window {
  display: flex;
  flex-direction: column;
  width: 1440px;
  height: 900px;
  background: var(--bg);
}
.mac-titlebar {
  height: 38px;
  background: #ffffff;
  border-bottom: 1px solid #e3e6ef;
  display: flex;
  align-items: center;
  padding: 0 16px;
  position: relative;
  flex-shrink: 0;
}
.traffic-lights {
  display: flex;
  gap: 8px;
}
.tl {
  width: 12px;
  height: 12px;
  border-radius: 50%;
}
.tl.red { background: #ff5f57; border: 1px solid #e0443e; }
.tl.yellow { background: #febc2e; border: 1px solid #d89e24; }
.tl.green { background: #28c840; border: 1px solid #1aab29; }
.mac-win-title {
  position: absolute;
  left: 50%;
  transform: translateX(-50%);
  font-size: 13px;
  font-weight: 600;
  color: #4a4f62;
}
.tree .row {
  height: 34px;
  font-size: 13.5px;
  border-bottom: 1px solid #f1f2f7;
}
.tree .row.selected {
  background: #eef2ff;
}
</style>
</head>
<body>
  <div class="mac-window">
    <div class="mac-titlebar">
      <div class="traffic-lights">
        <span class="tl red"></span>
        <span class="tl yellow"></span>
        <span class="tl green"></span>
      </div>
      <div class="mac-win-title">${titlebarText}</div>
    </div>
    ${innerBody}
  </div>
</body>
</html>`;
}

const topbarOpen = `
<header class="topbar">
  <div class="brand">
    <img class="logo" src="data:image/png;base64,${logoB64}" alt="" width="40" height="40" />
    <div>
      <h1>Ziplark</h1>
      <p class="tagline">Free · fast · every archive</p>
    </div>
  </div>
  <div class="topbar-right">
    <nav class="tabs">
      <button class="tab active">Open</button>
      <button class="tab">Create</button>
    </nav>
    <select class="lang-select">
      <option>EN</option>
    </select>
  </div>
</header>`;

const topbarCreate = `
<header class="topbar">
  <div class="brand">
    <img class="logo" src="data:image/png;base64,${logoB64}" alt="" width="40" height="40" />
    <div>
      <h1>Ziplark</h1>
      <p class="tagline">Free · fast · every archive</p>
    </div>
  </div>
  <div class="topbar-right">
    <nav class="tabs">
      <button class="tab">Open</button>
      <button class="tab active">Create</button>
    </nav>
    <select class="lang-select">
      <option>EN</option>
    </select>
  </div>
</header>`;

const rowsHtml = [
  { depth: 0, dir: true, open: true, name: 'crates', mtime: '2026-11-12 18:40', size: '—', checked: true },
  { depth: 1, dir: true, open: true, name: 'ziplark-core', mtime: '2026-11-12 18:40', size: '—', checked: true },
  { depth: 2, dir: true, open: true, name: 'src', mtime: '2026-11-12 18:39', size: '—', checked: true },
  { depth: 3, dir: false, name: 'lib.rs', mtime: '2026-11-12 18:39', size: '42.8 KB', checked: true },
  { depth: 3, dir: false, name: 'formats/zip.rs', mtime: '2026-11-12 18:35', size: '31.4 KB', checked: false },
  { depth: 3, dir: false, name: 'formats/sevenz.rs', mtime: '2026-11-12 17:50', size: '24.1 KB', checked: false },
  { depth: 3, dir: false, name: 'formats/rar.rs', mtime: '2026-11-12 17:12', size: '28.9 KB', checked: false },
  { depth: 3, dir: false, name: 'formats/tar.rs', mtime: '2026-11-12 16:48', size: '19.6 KB', checked: false },
  { depth: 3, dir: false, name: 'formats/iso.rs', mtime: '2026-11-12 16:20', size: '15.2 KB', checked: false },
  { depth: 2, dir: false, name: 'Cargo.toml', mtime: '2026-11-12 18:40', size: '2.4 KB', checked: false, selected: true },
  { depth: 0, dir: true, open: true, name: 'docs', mtime: '2026-11-12 15:10', size: '—', checked: false },
  { depth: 1, dir: false, name: 'architecture.md', mtime: '2026-11-12 15:10', size: '14.7 KB', checked: false },
  { depth: 1, dir: false, name: 'formats-matrix.md', mtime: '2026-11-12 14:55', size: '9.3 KB', checked: false },
  { depth: 0, dir: false, name: 'CHANGELOG.md', mtime: '2026-11-12 19:00', size: '8.1 KB', checked: false },
  { depth: 0, dir: false, name: 'README.md', mtime: '2026-11-12 19:00', size: '11.5 KB', checked: false },
  { depth: 0, dir: false, name: 'LICENSE-MIT', mtime: '2026-11-01 10:00', size: '1.1 KB', checked: false },
].map((r) => `
  <div class="row ${r.selected ? 'selected' : ''}" style="padding-left:${8 + r.depth * 18}px">
    <span class="rcheck ${r.checked ? 'on' : ''}"></span>
    <span class="twisty ${r.dir ? (r.open ? 'open' : '') : 'empty'}">▶</span>
    <span class="ricon">${r.dir ? '📁' : '📄'}</span>
    <span class="rname">${r.name}</span>
    <span class="rmtime">${r.mtime}</span>
    <span class="rsize">${r.size}</span>
  </div>
`).join('');

const footerHtml = `
<footer class="appfoot">
  <span id="app-ver">Ziplark v0.5.0</span>
  <span class="sep">·</span>
  <span>MIT © 2026 doaipm</span>
</footer>`;

// Scene 1: Archive browser
const scene1 = frameHtml('Ziplark — ziplark-v0.5.0-source.zip', `
  ${topbarOpen}
  <main>
    <section class="view active">
      <div class="panel">
        <div class="panel-head">
          <div class="panel-title">
            <h2>ziplark-v0.5.0-source.zip</h2>
            <p class="muted small">ZIP · 48 entries · 18.4 MB uncompressed → 5.2 MB packed (72% saved)</p>
          </div>
          <div class="panel-actions">
            <button class="ghost">Verify</button>
            <button class="ghost">Convert…</button>
            <button class="ghost">Add files…</button>
            <button class="ghost">Remove selected (4)</button>
            <button class="ghost">Extract selected (4)…</button>
            <button class="ghost">Extract here</button>
            <button class="primary">Extract all…</button>
            <button class="ghost">Close</button>
          </div>
        </div>
        <div class="tree-toolbar">
          <div class="search-wrap">
            <input id="tree-search" type="search" value="" placeholder="Search or filter entries (e.g. *.rs, docs/)…" />
            <button class="ghost small-btn">Search contents</button>
            <span class="muted small">48 items</span>
          </div>
          <span class="muted small hint-dblclick">Double-click a file to preview (or nested archive to open)</span>
        </div>
        <div class="tree-head">
          <span class="col-check"></span>
          <span class="col-name sortable"><span>Name</span> <span class="sort-ind">▲</span></span>
          <span class="col-mtime sortable"><span>Modified</span></span>
          <span class="col-size sortable"><span>Size</span></span>
        </div>
        <div class="tree">
          <div>${rowsHtml}</div>
        </div>
      </div>
    </section>
  </main>
  ${footerHtml}
`);

// Scene 2: Quick Preview modal open
const scene2 = frameHtml('Ziplark — In-Memory Quick Preview', `
  ${topbarOpen}
  <main>
    <section class="view active">
      <div class="panel">
        <div class="panel-head">
          <div class="panel-title">
            <h2>ziplark-v0.5.0-source.zip</h2>
            <p class="muted small">ZIP · 48 entries · 18.4 MB uncompressed → 5.2 MB packed</p>
          </div>
          <div class="panel-actions">
            <button class="ghost">Verify</button>
            <button class="ghost">Convert…</button>
            <button class="ghost">Extract here</button>
            <button class="primary">Extract all…</button>
            <button class="ghost">Close</button>
          </div>
        </div>
        <div class="tree">
          <div>${rowsHtml}</div>
        </div>
      </div>
    </section>
  </main>
  <div class="modal">
    <div class="modal-card preview-card" style="width: 820px;">
      <div class="preview-head">
        <div>
          <h3>crates/ziplark-core/Cargo.toml</h3>
          <p class="muted small">2.4 KB · UTF-8 text · CRC32 8F4A2C19 · Previewed in memory</p>
        </div>
        <div class="modal-actions">
          <button class="ghost small-btn">Copy</button>
          <button class="ghost small-btn">Extract file…</button>
          <button class="primary small-btn">Close</button>
        </div>
      </div>
      <div class="preview-body">
        <pre class="preview-code">[package]
name = "ziplark-core"
version = "0.5.0"
edition = "2021"
license = "MIT"
description = "Unified archive engine for ZIP, RAR (v4/v5), 7z, TAR (gz/zst/xz/bz2/lz4), ISO, CAB, DEB, RPM"

[features]
default = ["zip", "sevenz", "tar", "rar", "iso", "containers"]
zip = ["dep:zip", "dep:flate2"]
sevenz = ["dep:sevenz-rust2"]
tar = ["dep:tar", "dep:zstd", "dep:xz2", "dep:bzip2", "dep:lz4_flex"]
rar = ["dep:unrar_sys"]
iso = ["dep:iso9660"]

[dependencies]
thiserror = "2.0"
serde = { version = "1.0", features = ["derive"] }
chardetng = "0.1"
encoding_rs = "0.8"</pre>
      </div>
    </div>
  </div>
  ${footerHtml}
`);

// Scene 3: Create Archive view
const scene3 = frameHtml('Ziplark — Create Archive', `
  ${topbarCreate}
  <main>
    <section class="view active">
      <div class="dropzone small">
        <div class="drop-inner">
          <p><strong>Drop files &amp; folders here</strong> <span>or</span> <button class="link">add…</button></p>
          <p class="muted small">Pack into ZIP (AES-256), 7z, TAR.ZST, TAR.GZ, TAR.XZ, TAR.BZ2, TAR.LZ4</p>
        </div>
      </div>
      <ul class="input-list">
        <li><span class="path">📁 /Users/alex/Projects/ziplark/crates/ziplark-core</span><button class="rm">×</button></li>
        <li><span class="path">📁 /Users/alex/Projects/ziplark/docs</span><button class="rm">×</button></li>
        <li><span class="path">📄 /Users/alex/Projects/ziplark/README.md (11.5 KB)</span><button class="rm">×</button></li>
        <li><span class="path">📄 /Users/alex/Projects/ziplark/CHANGELOG.md (8.1 KB)</span><button class="rm">×</button></li>
        <li><span class="path">📄 /Users/alex/Projects/ziplark/Cargo.toml (1.8 KB)</span><button class="rm">×</button></li>
        <li><span class="path">📄 /Users/alex/Projects/ziplark/LICENSE-MIT (1.1 KB)</span><button class="rm">×</button></li>
      </ul>
      <div class="create-options">
        <label><span>Format</span>
          <select><option>ZIP (.zip) — AES-256 / Deflate</option></select>
        </label>
        <label><span>Compression</span>
          <select><option>Best (smallest)</option></select>
        </label>
        <label><span>Password <span class="muted small">(ZIP / 7z)</span></span>
          <input type="password" value="••••••••••••" />
        </label>
        <label><span>Repeat password</span>
          <input type="password" value="••••••••••••" />
        </label>
      </div>
      <div class="create-foot">
        <button class="ghost">Clear</button>
        <button class="primary">Create archive…</button>
      </div>
    </section>
  </main>
  ${footerHtml}
`);

const scenes = [
  { name: 'mac-1.png', html: scene1 },
  { name: 'mac-2.png', html: scene2 },
  { name: 'mac-3.png', html: scene3 },
];

for (const s of scenes) {
  const tmpHtml = path.join(OUT_DIR, s.name.replace('.png', '.html'));
  const rawPng = path.join(OUT_DIR, s.name.replace('.png', '.raw.png'));
  const tmpBmp = path.join(OUT_DIR, s.name.replace('.png', '.bmp'));
  const outPng = path.join(OUT_DIR, s.name);

  fs.writeFileSync(tmpHtml, s.html);
  execFileSync(CHROME, [
    '--headless=new',
    '--disable-gpu',
    '--hide-scrollbars',
    '--window-size=1440,900',
    '--force-device-scale-factor=2',
    '--default-background-color=ffffffff',
    `--screenshot=${rawPng}`,
    `file://${tmpHtml}`,
  ], { stdio: 'ignore' });

  // Strip alpha channel via BMP round-trip so Apple accepts the screenshot (hasAlpha: no).
  execFileSync('sips', ['-s', 'format', 'bmp', rawPng, '--out', tmpBmp], { stdio: 'ignore' });
  execFileSync('sips', ['-s', 'format', 'png', tmpBmp, '--out', outPng], { stdio: 'ignore' });
  fs.unlinkSync(tmpHtml);
  fs.unlinkSync(rawPng);
  fs.unlinkSync(tmpBmp);
  console.log('✓ Generated', outPng);
}

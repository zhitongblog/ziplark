// App Store Connect API helpers (ES256 JWT + fetch wrapper) + screenshot uploader.
// The .p8 private key is read from ~/.appstoreconnect/private_keys and is NEVER committed.
import crypto from 'node:crypto';
import fs from 'node:fs';

export const CONFIG = {
  keyId:   process.env.ASC_KEY_ID   || 'H85Q4NJPVD',
  issuer:  process.env.ASC_ISSUER   || '21dd1b35-fb04-42f1-8ec0-d847838fa7b6',
  bundleId: 'app.ziplark.desktop',
  appId:   process.env.ASC_APP_ID   || '6834275634',
  keyPath: process.env.ASC_KEY_PATH || `${process.env.HOME}/.appstoreconnect/private_keys/AuthKey_${process.env.ASC_KEY_ID || 'H85Q4NJPVD'}.p8`,
};

const b64u = (b) => Buffer.from(b).toString('base64').replace(/=/g, '').replace(/\+/g, '-').replace(/\//g, '_');

export function token() {
  const p8 = fs.readFileSync(CONFIG.keyPath, 'utf8');
  const now = Math.floor(Date.now() / 1000);
  const header = b64u(JSON.stringify({ alg: 'ES256', kid: CONFIG.keyId, typ: 'JWT' }));
  const payload = b64u(JSON.stringify({ iss: CONFIG.issuer, iat: now, exp: now + 1080, aud: 'appstoreconnect-v1' }));
  const key = crypto.createPrivateKey(p8);
  const sig = crypto.sign('SHA256', Buffer.from(`${header}.${payload}`), { key, dsaEncoding: 'ieee-p1363' });
  return `${header}.${payload}.${b64u(sig)}`;
}

let JWT = token();
let mintedAt = Date.now();

export async function api(method, path, body) {
  for (let i = 0; ; i++) {
    if (Date.now() - mintedAt > 15 * 60 * 1000) { JWT = token(); mintedAt = Date.now(); }
    const r = await fetch('https://api.appstoreconnect.apple.com' + path, {
      method,
      headers: { Authorization: `Bearer ${JWT}`, 'Content-Type': 'application/json' },
      body: body ? JSON.stringify(body) : undefined,
    });
    const text = await r.text();
    let json = null; try { json = text ? JSON.parse(text) : null; } catch { /* ignore */ }
    if (r.status >= 300) {
      const detail = json?.errors?.[0]?.detail || json?.errors?.[0]?.title || r.status;
      if (r.status >= 500 && i < 4) { await new Promise((res) => setTimeout(res, 1500 * (i + 1))); continue; }
      throw new Error(`${method} ${path} -> ${r.status}: ${detail}`);
    }
    return json;
  }
}

export async function resolveAppId() {
  if (CONFIG.appId) return CONFIG.appId;
  const res = await api('GET', `/v1/apps?filter[bundleId]=${CONFIG.bundleId}`);
  const hit = res.data?.[0];
  if (!hit) {
    throw new Error(
      `No App Store Connect record found for ${CONFIG.bundleId}. ` +
      `Create it once at https://appstoreconnect.apple.com/apps ("+" -> New App -> macOS -> Bundle ID: ${CONFIG.bundleId}).`
    );
  }
  CONFIG.appId = hit.id;
  return hit.id;
}

export async function uploadScreenshot(setId, file) {
  const buf = fs.readFileSync(file);
  const name = file.split('/').pop();
  const res = await api('POST', '/v1/appScreenshots', {
    data: {
      type: 'appScreenshots',
      attributes: { fileName: name, fileSize: buf.length },
      relationships: { appScreenshotSet: { data: { type: 'appScreenshotSets', id: setId } } },
    },
  });
  const id = res.data.id;
  for (const op of res.data.attributes.uploadOperations || []) {
    const headers = {};
    (op.requestHeaders || []).forEach((h) => (headers[h.name] = h.value));
    const up = await fetch(op.url, { method: op.method, headers, body: buf.subarray(op.offset, op.offset + op.length) });
    if (!up.ok) throw new Error(`PUT ${name} -> ${up.status}`);
  }
  const md5 = crypto.createHash('md5').update(buf).digest('hex');
  await api('PATCH', `/v1/appScreenshots/${id}`, {
    data: { type: 'appScreenshots', id, attributes: { uploaded: true, sourceFileChecksum: md5 } },
  });
  return name;
}

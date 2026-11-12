// Register bundle id `app.ziplark.desktop` + create Mac App Store provisioning
// profile (`src-tauri/Ziplark.provisionprofile`) via App Store Connect API.
//
// Usage: node scripts/asc/setup-appstore.mjs
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const KEY_ID = process.env.ASC_KEY_ID || 'H85Q4NJPVD';
const ISSUER = process.env.ASC_ISSUER || '21dd1b35-fb04-42f1-8ec0-d847838fa7b6';
const KEY_PATH = `${process.env.HOME}/.appstoreconnect/private_keys/AuthKey_${KEY_ID}.p8`;
const BUNDLE = 'app.ziplark.desktop';

const b64u = (b) => Buffer.from(b).toString('base64').replace(/=/g, '').replace(/\+/g, '-').replace(/\//g, '_');
function token() {
  const p8 = fs.readFileSync(KEY_PATH, 'utf8');
  const now = Math.floor(Date.now() / 1000);
  const header = b64u(JSON.stringify({ alg: 'ES256', kid: KEY_ID, typ: 'JWT' }));
  const payload = b64u(JSON.stringify({ iss: ISSUER, iat: now, exp: now + 1080, aud: 'appstoreconnect-v1' }));
  const key = crypto.createPrivateKey(p8);
  const sig = crypto.sign('SHA256', Buffer.from(`${header}.${payload}`), { key, dsaEncoding: 'ieee-p1363' });
  return `${header}.${payload}.${b64u(sig)}`;
}
const JWT = token();

async function api(method, urlPath, body) {
  const r = await fetch('https://api.appstoreconnect.apple.com' + urlPath, {
    method,
    headers: { Authorization: `Bearer ${JWT}`, 'Content-Type': 'application/json' },
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await r.text();
  const json = text ? JSON.parse(text) : null;
  if (r.status >= 300) {
    throw new Error(`${method} ${urlPath} -> ${r.status}: ${json?.errors?.[0]?.detail || json?.errors?.[0]?.title || text}`);
  }
  return json;
}

// 1. Register Bundle ID (`app.ziplark.desktop`)
let bundleId;
const existing = await api('GET', `/v1/bundleIds?filter[identifier]=${BUNDLE}`);
const hit = (existing.data || []).find((b) => b.attributes.identifier === BUNDLE);
if (hit) {
  bundleId = hit.id;
  console.log('✓ Bundle ID exists:', hit.id, hit.attributes.identifier, hit.attributes.platform);
} else {
  const res = await api('POST', '/v1/bundleIds', {
    data: {
      type: 'bundleIds',
      attributes: {
        identifier: BUNDLE,
        name: 'Ziplark',
        platform: 'MAC_OS',
      },
    },
  });
  bundleId = res.data.id;
  console.log('✓ Bundle ID created:', bundleId, BUNDLE);
}

// 2. Find Mac App Store / Apple Distribution certificate
const certs = await api('GET', '/v1/certificates?limit=200');
const macAppCerts = (certs.data || []).filter(
  (c) =>
    ['MAC_APP_DISTRIBUTION', 'DISTRIBUTION'].includes(c.attributes.certificateType) &&
    new Date(c.attributes.expirationDate) > new Date()
);
console.log(
  '• Certificates:',
  certs.data.map((c) => `${c.attributes.certificateType}:${c.id.slice(0, 6)}(${c.attributes.name})`).join(' | ')
);

if (!macAppCerts.length) {
  throw new Error('No active MAC_APP_DISTRIBUTION / DISTRIBUTION certificate found in ASC');
}

// 3. Create or refresh MAC_APP_STORE provisioning profile
const profileName = 'Ziplark MAS';
const exProfiles = await api('GET', `/v1/profiles?filter[name]=${encodeURIComponent(profileName)}`);
for (const p of exProfiles.data || []) {
  console.log(`• Deleting existing profile "${profileName}" (${p.id}) to recreate cleanly…`);
  await api('DELETE', `/v1/profiles/${p.id}`);
}

const profRes = await api('POST', '/v1/profiles', {
  data: {
    type: 'profiles',
    attributes: { name: profileName, profileType: 'MAC_APP_STORE' },
    relationships: {
      bundleId: { data: { type: 'bundleIds', id: bundleId } },
      certificates: { data: [{ type: 'certificates', id: macAppCerts[0].id }] },
    },
  },
});

const outPath = path.join(ROOT, 'src-tauri/Ziplark.provisionprofile');
fs.writeFileSync(outPath, Buffer.from(profRes.data.attributes.profileContent, 'base64'));
console.log(`✓ Wrote MAS provisioning profile -> ${outPath}`);

// 4. Check if App Store Connect app record already exists for `app.ziplark.desktop`
const apps = await api('GET', `/v1/apps?filter[bundleId]=${BUNDLE}`);
if (apps.data?.length) {
  console.log(`✓ ASC App record exists: appId=${apps.data[0].id} name="${apps.data[0].attributes.name}"`);
} else {
  console.log(
    `\nℹ Next one-time step in https://appstoreconnect.apple.com/apps :\n` +
      `  Click "+" -> "New App" -> Platform: macOS -> Name: Ziplark -> Bundle ID: ${BUNDLE} -> SKU: ziplark`
  );
}

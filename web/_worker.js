/**
 * Cloudflare Pages Advanced-mode worker.
 *
 * Handles /api/stats (live GitHub stars + total release downloads) and passes
 * everything else to the static asset server (env.ASSETS).
 *
 * Resilience: GitHub's anonymous API limit (60/hr) is shared across Cloudflare
 * edge IPs and gets exhausted, which used to make the numbers vanish. We now:
 *   1. cache a successful response for 5 min (edge cache), and
 *   2. keep a *last-known-good* copy for 7 days and serve it whenever the live
 *      GitHub fetch fails — so the counts never disappear once seen.
 * Set a GITHUB_TOKEN env var (read-only) to lift the limit to 5000/hr.
 */

const REPO = "zhitongblog/ziplark";
const TTL = 300; // fresh edge cache, seconds
const GOOD_TTL = 604800; // last-known-good retention, seconds (7 days)

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/api/stats") return stats(request, env);
    return env.ASSETS.fetch(request);
  },
};

function keys(request) {
  const base = new URL(request.url);
  base.search = "";
  const fresh = new Request(base.toString(), { method: "GET" });
  const good = new Request(base.toString() + "?_good=1", { method: "GET" });
  return { fresh, good };
}

function jsonResponse(payload, maxAge, extra) {
  return new Response(JSON.stringify(payload), {
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": `public, max-age=${Math.min(maxAge, 60)}, s-maxage=${maxAge}`,
      "access-control-allow-origin": "*",
      ...(extra || {}),
    },
  });
}

async function stats(request, env) {
  const cache = caches.default;
  const { fresh, good } = keys(request);

  const hit = await cache.match(fresh);
  if (hit) return hit;

  const headers = {
    "User-Agent": "Ziplark-stats-proxy",
    Accept: "application/vnd.github+json",
  };
  if (env && env.GITHUB_TOKEN) headers.Authorization = `Bearer ${env.GITHUB_TOKEN}`;

  let stars = null;
  let downloads = null;
  let latest_tag = null;
  let latest_url = null;
  try {
    const [repoRes, relRes] = await Promise.all([
      fetch(`https://api.github.com/repos/${REPO}`, { headers }),
      fetch(`https://api.github.com/repos/${REPO}/releases?per_page=100`, { headers }),
    ]);
    if (repoRes.ok) stars = (await repoRes.json()).stargazers_count ?? 0;
    if (relRes.ok) {
      const rels = await relRes.json();
      let total = 0;
      for (const r of rels) for (const a of r.assets || []) total += a.download_count || 0;
      downloads = total;
      const latest = rels.find((r) => !r.draft && !r.prerelease) || rels[0];
      if (latest) {
        latest_tag = latest.tag_name || null;
        latest_url = latest.html_url || null;
      }
    }
  } catch (_) {
    /* handled by fallback below */
  }

  // Success (we got at least the star count): cache fresh + last-known-good.
  if (stars !== null) {
    const payload = { stars, downloads, latest_tag, latest_url, fresh: true };
    const res = jsonResponse(payload, TTL);
    await cache.put(fresh, res.clone());
    await cache.put(good, jsonResponse(payload, GOOD_TTL));
    return res;
  }

  // Failure: serve the last-known-good copy if we have one.
  const lastGood = await cache.match(good);
  if (lastGood) {
    const payload = await lastGood.json();
    payload.fresh = false;
    // short cache so we retry GitHub soon, but still show real numbers now.
    return jsonResponse(payload, 60);
  }

  return jsonResponse({ stars: null, downloads: null, latest_tag: null, latest_url: null, fresh: false }, 30);
}

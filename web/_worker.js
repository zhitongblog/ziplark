/**
 * Cloudflare Pages Advanced-mode worker.
 *
 * Direct-upload Pages projects don't compile a `functions/` directory, so we
 * handle the one dynamic route (/api/stats) here and pass everything else to
 * the static asset server (env.ASSETS).
 *
 * /api/stats returns Ziplark's live GitHub stars + total release downloads,
 * edge-cached for 5 minutes so visitors never hit GitHub directly and the
 * numbers refresh without a redeploy. Set a GITHUB_TOKEN env var to lift the
 * GitHub rate limit from 60/hr (anon) to 5000/hr.
 */

const REPO = "zhitongblog/ziplark";
const CACHE_TTL = 300; // seconds

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/api/stats") return stats(request, env);
    return env.ASSETS.fetch(request);
  },
};

async function stats(request, env) {
  const cache = caches.default;
  const cacheKey = new Request(url(request), request);
  const hit = await cache.match(cacheKey);
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
    if (repoRes.ok) {
      const repo = await repoRes.json();
      stars = repo.stargazers_count ?? 0;
    }
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
    /* keep nulls; client falls back gracefully */
  }

  const res = new Response(
    JSON.stringify({ stars, downloads, latest_tag, latest_url, fresh: stars !== null }),
    {
      headers: {
        "content-type": "application/json; charset=utf-8",
        "cache-control": `public, max-age=60, s-maxage=${CACHE_TTL}`,
        "access-control-allow-origin": "*",
      },
    }
  );
  // Only cache successful upstream reads so failures retry quickly.
  if (stars !== null) await cache.put(cacheKey, res.clone());
  return res;
}

function url(request) {
  return new URL(request.url).toString();
}

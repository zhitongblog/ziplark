// One-shot App Store Connect metadata + build attachment + submission script for Ziplark (macOS).
//   node scripts/asc/release.mjs [--no-submit]
import { api, resolveAppId, uploadScreenshot } from './asc.mjs';
import * as M from './metadata.mjs';

const NO_SUBMIT = process.argv.includes('--no-submit');
const SKIP_SHOTS = process.argv.includes('--skip-screenshots');
const log = (...a) => console.log('•', ...a);

const APP = await resolveAppId();
log('Using ASC App ID:', APP);

// 1. App-level localizations
const appInfos = (await api('GET', `/v1/apps/${APP}/appInfos`)).data;
const appInfo =
  appInfos.find((d) => (d.attributes.appStoreState || d.attributes.state) === 'PREPARE_FOR_SUBMISSION') ||
  appInfos[0];
const ilocs = (await api('GET', `/v1/appInfos/${appInfo.id}/appInfoLocalizations?limit=50`)).data;

for (const [locale, a] of Object.entries(M.appInfoLocalizations)) {
  const ex = ilocs.find((l) => l.attributes.locale === locale);
  try {
    if (ex) {
      await api('PATCH', `/v1/appInfoLocalizations/${ex.id}`, {
        data: { type: 'appInfoLocalizations', id: ex.id, attributes: a },
      });
    } else {
      await api('POST', '/v1/appInfoLocalizations', {
        data: {
          type: 'appInfoLocalizations',
          attributes: { locale, ...a },
          relationships: { appInfo: { data: { type: 'appInfos', id: appInfo.id } } },
        },
      });
    }
    log('appInfo localization updated:', locale);
  } catch (e) {
    log('appInfo localization skipped:', locale, String(e.message).slice(0, 80));
  }
}

// 2. Category & Content Rights & 4+ Age Rating
try {
  await api('PATCH', `/v1/appInfos/${appInfo.id}`, {
    data: {
      type: 'appInfos',
      id: appInfo.id,
      relationships: {
        primaryCategory: { data: { type: 'appCategories', id: M.appFields.primaryCategory } },
      },
    },
  });
  await api('PATCH', `/v1/apps/${APP}`, {
    data: { type: 'apps', id: APP, attributes: { contentRightsDeclaration: M.appFields.contentRightsDeclaration } },
  });
  log('Category (UTILITIES) & content rights set');
} catch (e) {
  log('Category/content rights skipped:', String(e.message).slice(0, 80));
}

try {
  const decl = (await api('GET', `/v1/appInfos/${appInfo.id}/ageRatingDeclaration`)).data;
  if (decl) {
    await api('PATCH', `/v1/ageRatingDeclarations/${decl.id}`, {
      data: { type: 'ageRatingDeclarations', id: decl.id, attributes: M.ageRating },
    });
    log('Age rating 4+ configured');
  }
} catch (e) {
  log('Age rating skipped:', String(e.message).slice(0, 80));
}

// 3. Free ($0) pricing schedule
try {
  const sched = await api('GET', `/v1/appPriceSchedules/${APP}/manualPrices?include=appPricePoint&limit=1`);
  log('Price schedule already present (', sched.data.length, 'manual price(s) )');
} catch {
  const pts = (await api('GET', `/v1/apps/${APP}/appPricePoints?filter[territory]=USA&limit=3&include=territory`)).data;
  const free = pts.find((p) => Number(p.attributes.customerPrice) === 0);
  if (free) {
    await api('POST', '/v1/appPriceSchedules', {
      data: {
        type: 'appPriceSchedules',
        relationships: {
          app: { data: { type: 'apps', id: APP } },
          baseTerritory: { data: { type: 'territories', id: 'USA' } },
          manualPrices: { data: [{ type: 'appPrices', id: '${price1}' }] },
        },
      },
      included: [
        {
          type: 'appPrices',
          id: '${price1}',
          attributes: { startDate: null },
          relationships: { appPricePoint: { data: { type: 'appPricePoints', id: free.id } } },
        },
      ],
    });
    log('Free ($0) pricing schedule created');
  }
}

// 3b. Availability: all territories
try {
  const avail = await api('GET', `/v2/appAvailabilities/${APP}?include=territoryAvailabilities&limit[territoryAvailabilities]=1`);
  log('Availability already configured:', avail.data.attributes.availableInNewTerritories ? 'auto-new-territories' : 'manual');
} catch {
  const terrs = (await api('GET', '/v1/territories?limit=200')).data.map((t) => t.id);
  await api('POST', '/v2/appAvailabilities', {
    data: {
      type: 'appAvailabilities',
      attributes: { availableInNewTerritories: true },
      relationships: {
        app: { data: { type: 'apps', id: APP } },
        territoryAvailabilities: { data: terrs.map((_, i) => ({ type: 'territoryAvailabilities', id: `\${t${i}}` })) },
      },
    },
    included: terrs.map((t, i) => ({
      type: 'territoryAvailabilities',
      id: `\${t${i}}`,
      attributes: { available: true },
      relationships: { territory: { data: { type: 'territories', id: t } } },
    })),
  });
  log('Availability created: all', terrs.length, 'territories');
}

// 4. Version + version localizations + review detail + build attachment
const pCfg = M.platforms.MAC_OS;
const vers = (await api('GET', `/v1/apps/${APP}/appStoreVersions?filter[platform]=MAC_OS&limit=10`)).data;
let ver = vers.find((v) => v.attributes.appStoreState === 'PREPARE_FOR_SUBMISSION') || vers[0];

if (!ver) {
  ver = (
    await api('POST', '/v1/appStoreVersions', {
      data: {
        type: 'appStoreVersions',
        attributes: { platform: 'MAC_OS', versionString: pCfg.versionString, copyright: pCfg.copyright },
        relationships: { app: { data: { type: 'apps', id: APP } } },
      },
    })
  ).data;
} else {
  await api('PATCH', `/v1/appStoreVersions/${ver.id}`, {
    data: {
      type: 'appStoreVersions',
      id: ver.id,
      attributes: { versionString: pCfg.versionString, copyright: pCfg.copyright },
    },
  });
}
log('macOS version:', ver.id, pCfg.versionString);

const vlocs = (await api('GET', `/v1/appStoreVersions/${ver.id}/appStoreVersionLocalizations?limit=50`)).data;
const isFirstRelease = vers.length <= 1;

for (const [locale, l] of Object.entries(M.versionLocalizations)) {
  const ex = vlocs.find((x) => x.attributes.locale === locale);
  const attrs = { ...l };
  if (isFirstRelease) delete attrs.whatsNew;
  if (ex) {
    await api('PATCH', `/v1/appStoreVersionLocalizations/${ex.id}`, {
      data: { type: 'appStoreVersionLocalizations', id: ex.id, attributes: attrs },
    });
  } else {
    await api('POST', '/v1/appStoreVersionLocalizations', {
      data: {
        type: 'appStoreVersionLocalizations',
        attributes: { locale, ...attrs },
        relationships: { appStoreVersion: { data: { type: 'appStoreVersions', id: ver.id } } },
      },
    });
  }
  log('Version localization updated:', locale);
}

// Review details
try {
  const rd = (await api('GET', `/v1/appStoreVersions/${ver.id}/appStoreReviewDetail`)).data;
  if (rd) {
    await api('PATCH', `/v1/appStoreReviewDetails/${rd.id}`, {
      data: { type: 'appStoreReviewDetails', id: rd.id, attributes: M.reviewDetail },
    });
  } else {
    await api('POST', '/v1/appStoreReviewDetails', {
      data: {
        type: 'appStoreReviewDetails',
        attributes: M.reviewDetail,
        relationships: { appStoreVersion: { data: { type: 'appStoreVersions', id: ver.id } } },
      },
    });
  }
} catch {
  await api('POST', '/v1/appStoreReviewDetails', {
    data: {
      type: 'appStoreReviewDetails',
      attributes: M.reviewDetail,
      relationships: { appStoreVersion: { data: { type: 'appStoreVersions', id: ver.id } } },
    },
  });
}
log('Review details updated');

// 5. Upload screenshots into empty sets
if (!SKIP_SHOTS && M.screenshots?.MAC_OS) {
  const shots = M.screenshots.MAC_OS;
  const vlocs2 = (await api('GET', `/v1/appStoreVersions/${ver.id}/appStoreVersionLocalizations?limit=50`)).data;
  for (const vl of vlocs2) {
    if (!M.versionLocalizations[vl.attributes.locale]) continue;
    const allSets = (await api('GET', `/v1/appScreenshotSets/${vl.id}/appScreenshotSets`).catch(() => null))?.data
      || (await api('GET', `/v1/appStoreVersionLocalizations/${vl.id}/appScreenshotSets`)).data;
    for (const plan of shots.sets) {
      let set = allSets.find((s) => s.attributes.screenshotDisplayType === plan.displayType);
      if (!set) {
        set = (
          await api('POST', '/v1/appScreenshotSets', {
            data: {
              type: 'appScreenshotSets',
              attributes: { screenshotDisplayType: plan.displayType },
              relationships: { appStoreVersionLocalization: { data: { type: 'appStoreVersionLocalizations', id: vl.id } } },
            },
          })
        ).data;
      }
      const existing = (await api('GET', `/v1/appScreenshotSets/${set.id}/appScreenshots`)).data.length;
      if (existing > 0) {
        log('MAC_OS', plan.displayType, vl.attributes.locale, '— already has', existing, '(skip)');
        continue;
      }
      for (const f of plan.files) {
        await uploadScreenshot(set.id, shots.dir + f);
        log('  uploaded screenshot', vl.attributes.locale, plan.displayType, f);
      }
    }
  }
}

// Attach processed build if available
const builds = (
  await api(
    'GET',
    `/v1/builds?filter[app]=${APP}&filter[preReleaseVersion.platform]=MAC_OS&filter[version]=${pCfg.buildVersion}&limit=5`
  )
).data;
const validBuild = builds.find((b) => b.attributes.processingState === 'VALID');
if (validBuild) {
  await api('PATCH', `/v1/appStoreVersions/${ver.id}/relationships/build`, {
    data: { type: 'builds', id: validBuild.id },
  });
  log('Attached VALID build:', validBuild.id);
} else {
  log('Build not yet VALID in ASC (current states:', builds.map((b) => b.attributes.processingState).join(', ') || 'none', ')');
}

if (!NO_SUBMIT && validBuild) {
  const subs = (
    await api('GET', `/v1/apps/${APP}/reviewSubmissions?filter[platform]=MAC_OS&filter[state]=READY_FOR_REVIEW&limit=1`)
  ).data || [];
  const sub =
    subs[0] ||
    (
      await api('POST', '/v1/reviewSubmissions', {
        data: {
          type: 'reviewSubmissions',
          attributes: { platform: 'MAC_OS' },
          relationships: { app: { data: { type: 'apps', id: APP } } },
        },
      })
    ).data;
  const items = (await api('GET', `/v1/reviewSubmissions/${sub.id}/items`)).data || [];
  if (!items.length) {
    await api('POST', '/v1/reviewSubmissionItems', {
      data: {
        type: 'reviewSubmissionItems',
        relationships: {
          reviewSubmission: { data: { type: 'reviewSubmissions', id: sub.id } },
          appStoreVersion: { data: { type: 'appStoreVersions', id: ver.id } },
        },
      },
    });
  }
  await api('PATCH', `/v1/reviewSubmissions/${sub.id}`, {
    data: { type: 'reviewSubmissions', id: sub.id, attributes: { submitted: true } },
  });
  const after = (await api('GET', `/v1/reviewSubmissions/${sub.id}`)).data;
  log('✓ Submitted for Mac App Store review! State:', after.attributes.state);
}

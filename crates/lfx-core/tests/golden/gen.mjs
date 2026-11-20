// Golden-Vektoren für lfx-core, erzeugt aus dem Original-JS des Web-Themes (nur lesend, keine Netzwerkzugriffe).
//
//   LENZFLIX_WEB_DIR=<Wurzel des Web-Repos> node crates/lfx-core/tests/golden/gen.mjs [--out <datei>]
//
// Standard (ohne LENZFLIX_WEB_DIR): Web-Repo fünf Ebenen über dieser Datei (Elternrepo des Submoduls `Anwendung`).
// Alle Daten sind erfunden. Der Generator läuft mit TZ=UTC (Lokalzeit-Funktionen sind dann deterministisch).
import { pathToFileURL, fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';

process.env.TZ = 'UTC';
const here = path.dirname(fileURLToPath(import.meta.url));
const webDir = process.env.LENZFLIX_WEB_DIR || path.resolve(here, '../../../../..');
const outArg = process.argv.indexOf('--out');
const out = outArg > 0 ? process.argv[outArg + 1] : path.join(here, 'web.json');
if (!fs.existsSync(path.join(webDir, 'src/js/core/dayseed.js'))) {
  console.error(`Web-Repo nicht gefunden: ${webDir} (LENZFLIX_WEB_DIR setzen)`);
  process.exit(2);
}

// Minimale Browser-Attrappen (nur für Importe; es wird nie ein Netzwerkzugriff ausgelöst).
const store = new Map();
globalThis.window = {
  dispatchEvent: () => true,
  localStorage: {
    getItem: (k) => (store.has(k) ? store.get(k) : null),
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
  },
  ApiClient: { serverAddress: () => 'http://mock.invalid', getCurrentUserId: () => 'u1', accessToken: () => 'x', serverId: () => 's1' },
};
const imp = (f) => import(pathToFileURL(path.join(webDir, 'src/js', f)).href);
const seed = await imp('core/dayseed.js');
const logic = await imp('features/rows/logic.js');
const bb = await imp('features/billboard/format.js');
const npf = await imp('features/newpopular/format.js');
const nc = await imp('shared/newcontent.js');
const api = await imp('core/api.js');
const imgscale = await imp('core/imgscale.js');
const routes = await imp('shared/routes.js');
const grid = await imp('features/library/grid.js');
const trailer = await imp('shared/trailer.js');

const g = {};
const rnd = seed.seededRandom('lfx-core-golden');
const pick = (arr) => arr[Math.floor(rnd() * arr.length)];

// ---- dayseed ---------------------------------------------------------------
const strings = ['', 'a', 'abc', '2026-11-20', '2026-11-20:series', 'Komödie', 'Ärger & Öl', 'Straße', 'İstanbul', '日本語', '😀 Emoji', 'Zebra Ü', 'x'.repeat(200)];
g.hash = strings.map((s) => [s, seed.hashString(s)]);
g.random = ['2026-11-20', 'Komödie', '😀', 'a', ''].map((s) => {
  const r = seed.seededRandom(s);
  return [s, Array.from({ length: 8 }, () => r())];
});
const seeds = ['2026-11-20', '2026-11-21:movies', 'Ärger', '😀x', ''];
g.shuffle = [];
for (const n of [0, 1, 2, 3, 5, 12, 40]) {
  for (const s of seeds) {
    const list = Array.from({ length: n }, (_, i) => i);
    const a = seed.seededShuffle(list, s);
    const b = logic.seededShuffle(list, s);
    if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error('dayseed und logic weichen ab');
    g.shuffle.push({ n, seed: s, out: a });
  }
}
const genreSets = [
  ['Action', 'Komödie', 'Drama', 'Krimi', 'Science-Fiction', 'Animation', 'Dokumentarfilm', 'Familie', 'Abenteuer', 'Horror'],
  ['Action & Adventure', 'Sci-Fi & Fantasy', 'Äktschn', 'Zeichentrick', 'zeitgeschichte', 'Märchen', 'Musik', 'Mystery', 'TV-Film', 'Western', 'Krieg', 'Ökologie', 'Straße', 'Strasse', 'Drama', 'drama'],
  ['B', 'a', 'A', 'b', 'Ab', 'ab', 'aB', 'a b', 'a-b', 'a_b', '1', '10', '2', 'Z', 'ä', 'ß', 'ss'],
];
g.pickGenres = [];
for (const names of genreSets) {
  for (const s of ['2026-11-20', '2026-11-21', '2026-11-20:movies', 'Ärger']) {
    for (const count of [3, 6]) g.pickGenres.push({ input: names, seed: s, count, out: logic.pickGenres(names, { seed: s, count }) });
  }
}
const countInput = [
  { Name: 'Gross', MovieCount: 9, SeriesCount: 0 }, { Name: 'Klein', MovieCount: 3, SeriesCount: 4 }, { Name: 'Mix', MovieCount: 2, SeriesCount: 6 },
  { Name: 'OhneZahl' }, { Name: 'Gross' }, { Name: 'Null', MovieCount: 0 }, 'Text', { Name: '' }, { Name: 'Ünter', SeriesCount: 8 },
];
g.pickGenresObjects = ['2026-11-20', 'x'].map((s) => ({ input: countInput, seed: s, count: 6, out: logic.pickGenres(countInput, { seed: s, count: 6 }) }));
g.dayKey = [[2026, 11, 20], [2026, 1, 5], [2027, 12, 31], [2024, 2, 29], [999, 3, 4]].map(([y, m, d]) => [y, m, d, seed.dayKey(new Date(Date.UTC(y, m - 1, d, 12)))]);

// ---- erfundene Items --------------------------------------------------------
const NOW = Date.UTC(2026, 10, 20, 23, 30);
const days = (n) => new Date(NOW - n * 86400000).toISOString().replace('Z', '0000Z').replace(/\.(\d{3})0000Z/, '.$10000Z');
const types = ['Movie', 'Series', 'Episode', 'Season', 'BoxSet'];
function makeItem(i) {
  const t = pick(types);
  const it = { Id: `id${i}`, Name: `Titel ${i}`, Type: t };
  if (rnd() < 0.9) it.DateCreated = days(Math.floor(rnd() * 400));
  if (rnd() < 0.15) it.DateCreated = 'kaputt';
  if (rnd() < 0.5) it.PremiereDate = days(Math.floor(rnd() * 900));
  if (rnd() < 0.7) it.Genres = ['Drama', 'Krimi', 'Komödie'].slice(0, 1 + Math.floor(rnd() * 3));
  if (rnd() < 0.7) it.ProductionYear = 1980 + Math.floor(rnd() * 46);
  if (rnd() < 0.8) it.RunTimeTicks = Math.floor(rnd() * 200) * 600000000 + Math.floor(rnd() * 600000000);
  if (rnd() < 0.5) it.ChildCount = Math.floor(rnd() * 6);
  if (rnd() < 0.7) it.OfficialRating = pick(['FSK-12', 'FSK 16', 'DE-6', 'PG-13', 'FSK-0', 'de 18', 'FSK-123', '']);
  it.ImageTags = {};
  if (rnd() < 0.8) it.ImageTags.Primary = `p${i}`;
  if (rnd() < 0.6) it.ImageTags.Logo = `l${i}`;
  if (rnd() < 0.3) it.ImageTags.Thumb = `t${i}`;
  if (rnd() < 0.7) it.BackdropImageTags = [`b${i}`];
  if (rnd() < 0.3) { it.ParentThumbItemId = `pt${i}`; it.ParentThumbImageTag = `ptt${i}`; }
  if (rnd() < 0.3) { it.ParentBackdropItemId = `pb${i}`; it.ParentBackdropImageTags = [`pbt${i}`]; }
  if (rnd() < 0.3) { it.ParentLogoItemId = `pl${i}`; it.ParentLogoImageTag = `plt${i}`; }
  if (rnd() < 0.4) { it.SeriesId = `ser${Math.floor(rnd() * 5)}`; it.SeriesPrimaryImageTag = `sp${i}`; }
  if (rnd() < 0.5) it.IndexNumber = 1 + Math.floor(rnd() * 3);
  if (rnd() < 0.5) it.ParentIndexNumber = 1 + Math.floor(rnd() * 3);
  if (rnd() < 0.5) it.LocalTrailerCount = Math.floor(rnd() * 3);
  if (rnd() < 0.6) it.UserData = { PlaybackPositionTicks: Math.floor(rnd() * 3) * 100000000, PlayCount: Math.floor(rnd() * 3), PlayedPercentage: rnd() < 0.5 ? Math.floor(rnd() * 150) - 10 : undefined };
  return JSON.parse(JSON.stringify(it));
}
const items = Array.from({ length: 160 }, (_, i) => makeItem(i));
g.now = NOW;
g.items = items;

// ---- billboard/format ---------------------------------------------------------
g.formatRuntime = [0, 1, 299999999, 300000000, 600000000, 35400000000, 36000000000, 72000000000, 63600000000, -5, 6.5e11].map((t) => [t, bb.formatRuntime(t)]);
g.formatRuntime.push([null, bb.formatRuntime(null)]);
g.formatRating = ['FSK-12', 'fsk 6', 'DE-16', 'de 18', 'FSK-123', 'PG-13', '', '  FSK-0 ', 'DE', 'FSK', 'FSK--7', 'FSK\t9'].map((r) => [r, bb.formatRating(r)]);
g.formatSeasons = [0, 1, 2, 7, -1].map((n) => [n, bb.formatSeasons(n)]);
g.metaParts = items.filter((i) => ['Movie', 'Series'].includes(i.Type)).slice(0, 40).map((i) => [i, bb.metaParts(i)]);
g.genreLine = [[], ['A'], ['A', 'B', 'C', 'D'], ['', 'B']].map((gl) => [gl, bb.genreLine({ Genres: gl })]);
g.bbBadge = items.slice(0, 60).map((i) => [i.Id, bb.badgeFor(i, NOW)]);
g.hasProgress = items.slice(0, 60).map((i) => [i.Id, bb.hasProgress(i)]);
g.saverPool = [3, 5, 9].flatMap((n) => {
  const pool = items.slice(0, n);
  return [null, 'id0', 'id7', 'nope'].map((k) => ({ n, keep: k, out: bb.saverPool(pool, k).map((i) => i.Id) }));
});
g.buildPool = [];
for (const sd of ['a', 'b', 'c', 'd']) {
  for (const avoid of [null, 'id3', 'id5']) {
    const latest = items.slice(0, 40);
    const random = items.slice(40, 100);
    const r = seed.seededRandom(sd);
    const pool = bb.buildPool(latest, random, { avoidFirstId: avoid, rng: r });
    const r2 = seed.seededRandom(sd);
    const lib = bb.buildPool(latest, random, { newCount: 3, randomCount: 7, rng: r2 });
    g.buildPool.push({ seed: sd, avoid, out: pool.map((i) => i.Id), lib: lib.map((i) => i.Id) });
  }
}

// ---- rows/logic -----------------------------------------------------------------
g.isRecent = items.slice(0, 80).map((i) => [i.Id, logic.isRecent(i.DateCreated, NOW, 14), logic.isRecent(i.PremiereDate, NOW, 365)]);
const newSet = logic.newSeriesIdsFrom(items, NOW);
g.newSeriesIds = [...newSet].sort();
g.rowBadge = items.slice(0, 80).map((i) => [i.Id, logic.badgeFor(i, newSet, NOW)]);
g.top10 = [items, items.slice(0, 30), items.slice(100)].map((l) => logic.selectTop10(l).map((i) => i.Id));
g.progress = items.map((i) => [i.Id, logic.progressPercent(i)]);

// ---- newpopular --------------------------------------------------------------------
g.dateParts = items.slice(0, 30).map((i) => [i.Id, npf.dateParts(i.DateCreated)]);
g.sinceLabel = items.slice(0, 60).map((i) => [i.Id, npf.sinceLabel(i, newSet, NOW)]);
g.uniqueTitles = [3, 10, 50].map((n) => ({ n, out: npf.uniqueTitles([...items, ...items.slice(0, 20)], n).map((i) => i.Id) }));
g.hasPlayCounts = [items.slice(0, 5), items.slice(0, 40), []].map((l) => npf.hasPlayCounts(l));
const info = { bySeries: nc.newContentFrom(items, NOW), newMovieIds: new Set(), newSeriesIds: new Set() };
for (const i of items) {
  if (!logic.isRecent(i.DateCreated, NOW)) continue;
  if (i.Type === 'Movie') info.newMovieIds.add(i.Id);
  if (i.Type === 'Series') info.newSeriesIds.add(i.Id);
}
g.newContent = Object.fromEntries([...info.bySeries].sort());
g.newBadges = items.slice(0, 100).map((i) => [i.Id, nc.newBadge(i, info), nc.plusBadge(i, info)]);
g.newMovieIds = [...info.newMovieIds].sort();
g.newSeriesIdsInfo = [...info.newSeriesIds].sort();

// ---- Bilder -----------------------------------------------------------------------------
const kindOf = (t) => t;
g.landscape = items.slice(0, 100).map((i) => [i.Id, api.pickLandscape(i)]);
const widthOf = (url) => { const m = /[?&]maxWidth=(\d+)/.exec(url || ''); return m ? Number(m[1]) : null; };
g.logo = items.slice(0, 100).map((i) => {
  const u = api.logoImage(i, { maxWidth: 400 });
  return [i.Id, u ? { url: u.replace('http://mock.invalid', ''), width: widthOf(u) } : null];
});
g.imageScale = [];
for (const v of [0.1, 0.25, 0.333, 2 / 3, 0.7, 1, 1.234, 1.9, 2, 3, 600]) {
  api.setImageScale(v);
  g.imageScale.push({ scale: v, widths: [64, 300, 400, 420, 600, 720, 1000, 1600].map((w) => widthOf(api.imageUrlFor('x', { maxWidth: w }))) });
}
api.setImageScale(1);
g.imgscaleStore = [];
for (const [normal, saver] of [[undefined, undefined], [3, 0.1], [0.4, 2], [1.234, 0.333], [0.5, 0.25], [1.17, 0.83], [NaN, 0.5]]) {
  window.localStorage.removeItem('lfx.imgscale.u1');
  imgscale.setImageScales({ normal, saver });
  g.imgscaleStore.push({ in: [normal ?? null, saver ?? null], normal: imgscale.getNormalScale(), saver: imgscale.getSaverScale() });
}

// ---- Routen / Raster --------------------------------------------------------------------
const paramsOf = (q) => ({ path: '/home', params: new URLSearchParams(q) });
g.libraryHref = [['series', {}], ['movies', { genre: 'a b' }], ['series', { view: 'grid', sort: 'za' }], ['movies', { genre: 'ä&x', view: 'grid', sort: 'year' }], ['series', { sort: 'bogus' }], ['x', {}]]
  .map(([k, o]) => [k, o, routes.libraryHref(k, o)]);
g.myListHref = [{}, { type: 'movies' }, { type: 'series', sort: 'new' }, { sort: 'za' }, { type: 'x', sort: 'year' }].map((o) => [o, routes.myListHref(o)]);
g.parseRoute = ['lfx=series', 'lfx=movies&genre=a%20b&view=grid&sort=za', 'lfx=series&sort=bogus&view=x', 'lfx=mylist&type=series&sort=new', 'lfx=mylist&type=zzz', 'lfx=title&id=1', 'lfx=series&genre=%C3%A4%2B+x']
  .map((q) => [q, routes.parseLfxRoute(paramsOf(q))]);
g.sortParams = [['az', 'Series'], ['za', 'Movie'], ['new', 'Series'], ['new', 'Movie'], ['year', 'Movie'], [undefined, 'Movie'], ['bogus', 'Series']]
  .map(([s, t]) => [s ?? null, t, grid.sortParams(s, { IncludeItemTypes: t })]);

// ---- Trailer -----------------------------------------------------------------------------
const remote = [['https://a.example/x.mp4'], ['http://a.example/x.mp4'], ['https://a.example/watch?v=1'], ['https://a.example/x.webm?t=1'], ['https://a.example/x.M4V#f'], ['https://a.example/x.mp4s'], ['https://a.example/x.mp4x', 'https://b.example/y.mp4'], []];
g.remoteTrailer = remote.map((urls) => [urls, trailer.directRemoteTrailerUrl({ RemoteTrailers: urls.map((Url) => ({ Url })) })]);

fs.writeFileSync(out, JSON.stringify(g));
console.log(`geschrieben: ${path.relative(process.cwd(), out)} (${(fs.statSync(out).size / 1024).toFixed(0)} KiB)`);

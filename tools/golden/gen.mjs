// Golden-Vektoren aus dem ORIGINAL-JS-Planer (src/js/tv/schedule/*.js des Web-Themes) für den Rust-Port.
// Aufruf: node tools/golden/gen.mjs [--out <Ordner>]
// Umgebung: LENZFLIX_WEB_DIR = Wurzel des Web-Repos (Standard: drei Ordner über diesem Skript),
//           GOLDEN_OUT = Ausgabeordner (Standard: crates/lfx-tv-schedule/tests/golden).
// Alle Daten sind synthetisch (erfundene Titel). Ausgabe ist deterministisch (eigener Seed, feste Reihenfolge, TZ=UTC).
// JSON-Konventionen: siehe README.md (Marker {"$": ...} für undefined/NaN/±Infinity/-0/lone Surrogates).
process.env.TZ = process.env.GOLDEN_TZ_TEST || 'UTC'; // vor der ersten Date-Nutzung; Vektoren sind zeitzonenunabhängig (selftest.mjs prüft das mit GOLDEN_TZ_TEST)

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { execFileSync } from 'node:child_process';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const WEB = path.resolve(process.env.LENZFLIX_WEB_DIR || path.resolve(HERE, '../../..'));
const argOut = process.argv.indexOf('--out') >= 0 ? process.argv[process.argv.indexOf('--out') + 1] : null;
const OUT = path.resolve(argOut || process.env.GOLDEN_OUT || path.resolve(HERE, '../../crates/lfx-tv-schedule/tests/golden'));
const SCHED = path.join(WEB, 'src/js/tv/schedule');
const imp = (rel) => import(pathToFileURL(path.join(SCHED, rel)).href);

const index = await imp('index.js');
const T = await imp('time.js');
const H = await imp('hash.js');
const R = await imp('rating.js');
const C = await imp('catalog.js');
const P = await imp('pool.js');
const F = await imp('fill.js');
const cfgMod = await import(pathToFileURL(path.join(WEB, 'src/js/features/tvdata/config.js')).href);

// KEEP-Liste aus features/tvdata/data.js (Kopie; Abgleich per Import, falls das Modul ohne Browser ladbar ist)
const KEEP = {
  movies: ['Id', 'Name', 'RunTimeTicks', 'Genres', 'OfficialRating', 'ProductionYear', 'PremiereDate', 'DateCreated', 'LocalTrailerCount', 'ImageTags', 'BackdropImageTags'],
  series: ['Id', 'Name', 'Genres', 'OfficialRating', 'ProductionYear', 'DateCreated', 'ImageTags', 'BackdropImageTags'],
  episodes: ['Id', 'Name', 'SeriesId', 'SeriesName', 'ParentIndexNumber', 'IndexNumber', 'RunTimeTicks', 'PremiereDate', 'DateCreated', 'OfficialRating'],
  boxsets: ['Id', 'Name', 'ChildCount'],
};
let keepChecked = false;
try {
  const dm = await import(pathToFileURL(path.join(WEB, 'src/js/features/tvdata/data.js')).href);
  if (JSON.stringify(dm.KEEP) !== JSON.stringify(KEEP)) throw new Error('KEEP in data.js weicht von der Kopie in gen.mjs ab');
  keepChecked = true;
} catch (e) {
  if (String(e.message).startsWith('KEEP in data.js')) throw e;
}

// ---------------------------------------------------------------------------------------------- JSON-Kodierung

const isWellFormed = (s) => (typeof s.isWellFormed === 'function' ? s.isWellFormed() : !/[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/.test(s));
function enc(v) {
  if (v === undefined) return { $: 'undefined' };
  if (v === null || typeof v === 'boolean') return v;
  if (typeof v === 'number') {
    if (Number.isNaN(v)) return { $: 'NaN' };
    if (v === Infinity) return { $: 'Infinity' };
    if (v === -Infinity) return { $: '-Infinity' };
    if (Object.is(v, -0)) return { $: '-0' };
    return v;
  }
  if (typeof v === 'string') {
    if (isWellFormed(v)) return v;
    const u = [];
    for (let i = 0; i < v.length; i++) u.push(v.charCodeAt(i));
    return { $: 'u16', u };
  }
  if (Array.isArray(v)) return v.map(enc);
  if (typeof v === 'object') {
    const o = {};
    for (const k of Object.keys(v)) if (v[k] !== undefined) o[k] = enc(v[k]);
    return o;
  }
  throw new Error('nicht kodierbar: ' + typeof v);
}

const written = [];
function writeJson(name, obj) {
  const s = JSON.stringify(enc(obj)) + '\n';
  fs.writeFileSync(path.join(OUT, name), s);
  written.push({ name, bytes: Buffer.byteLength(s) });
  if (Buffer.byteLength(s) > 4.8e6) throw new Error('Datei zu groß: ' + name);
}
fs.mkdirSync(OUT, { recursive: true });
for (const f of fs.readdirSync(OUT)) if (f.endsWith('.json')) fs.unlinkSync(path.join(OUT, f)); // alte Läufe entfernen

// ---------------------------------------------------------------------------------------------- eigener PRNG (splitmix32)

function rng(seed) {
  let s = seed >>> 0;
  const next = () => {
    s = (s + 0x9e3779b9) >>> 0;
    let z = s;
    z = Math.imul(z ^ (z >>> 16), 0x21f0aaad);
    z = Math.imul(z ^ (z >>> 15), 0x735a2d97);
    return ((z ^ (z >>> 15)) >>> 0) / 4294967296;
  };
  const r = {
    f: next,
    int: (n) => Math.floor(next() * n),
    range: (a, b) => a + Math.floor(next() * (b - a + 1)),
    chance: (p) => next() < p,
    pick: (arr) => arr[Math.floor(next() * arr.length)],
    shuffle: (arr) => { for (let i = arr.length - 1; i > 0; i--) { const j = Math.floor(next() * (i + 1)); [arr[i], arr[j]] = [arr[j], arr[i]]; } return arr; },
    hex: (n) => { let o = ''; for (let i = 0; i < n; i++) o += '0123456789abcdef'[Math.floor(next() * 16)]; return o; },
    weighted: (pairs) => { let t = 0; for (const p of pairs) t += p[1]; let x = next() * t; for (const p of pairs) { x -= p[1]; if (x < 0) return p[0]; } return pairs[pairs.length - 1][0]; },
  };
  return r;
}

// ---------------------------------------------------------------------------------------------- (a) Primitive

const WIN = { D0: T.dayOfDate(2026, 10, 1) };

function primitives() {
  const r = rng(101);
  const out = {};

  // --- Hash
  const strs = ['', 'a', 'abc', 'Film 0001', 'Straße', 'STRASSE', 'İstanbul', 'ΑΣ', 'ας', 'é', 'é', 'Ünï', '日本語', '😀', 'a😀b', '￿', '\u0001', 'x\u0001y', 'fill\u0001kino', 'Z', 'a', 'A', 'ab', 'abc\u0000def', '\u{10000}', '\u{10ffff}'];
  for (let i = 0; i < 40; i++) strs.push(r.hex(r.range(1, 40)));
  const units = [[0xd800], [0xdc00], [0xdc00, 0xd800], [0x61, 0xd83d], [0xd83d, 0xde00], [0xd83d, 0x62], [0xffff], [0xfffd]];
  const fromU = (u) => String.fromCharCode(...u);
  out.fnv1a = strs.map((s) => ({ in: s, out: H.fnv1a(s) })).concat(units.map((u) => ({ in: fromU(u), out: H.fnv1a(fromU(u)) })));
  const u32 = [0, 1, 2, 0x7fffffff, 0x80000000, 0x80000001, 0xffffffff, 0xdeadbeef, 0x811c9dc5, 0x85ebca6b, 0x55555555, 0xaaaaaaaa, 0x0000ffff, 0xffff0000];
  for (let i = 0; i < 60; i++) u32.push(Math.floor(r.f() * 4294967296));
  out.fmix32 = u32.map((x) => ({ in: x, out: H.fmix32(x) }));
  out.bitrev32 = u32.map((x) => ({ in: x, out: H.bitrev32(x) }));
  out.mulberry32 = [0, 1, 42, 0x7fffffff, 0x80000000, 0xffffffff, 0xdeadbeef, 123456789, 4294967296 + 5, -1, -123456, 2 ** 40 + 7]
    .map((seed) => { const g = H.mulberry32(seed); return { seed, out: Array.from({ length: 24 }, () => g()) }; });
  const keys = ['fill\u0001', 'fill\u0001kino', 'kino', 'klassik', 'genre', 'genre:aktion', 'serie', 'marathon', 'premiere', 'themeday', 'themeday:boxset', 'themeday:decade:1990', 'themeday:overlength', 'themeday:pilot', 'fill\u00015', ''];
  out.keyHash = [];
  for (const k of keys) for (const id of ['', '0', 'a', r.hex(16), r.hex(32), '2026-11-01:3', '12:boxset', 'é', '😀', '2830:serie']) out.keyHash.push({ key: k, id, out: H.keyHash(k, id) });
  for (const u of units) out.keyHash.push({ key: 'k', id: fromU(u), out: H.keyHash('k', fromU(u)) });
  // cmpStr: Code-Unit-Reihenfolge (nicht Codepoint/UTF-8!) – Paare mit Surrogat-Fallen
  const cmpS = ['', 'a', 'b', 'ab', 'a\u0000', 'A', 'Z', 'z', 'é', 'e', '￿', '퟿', '', '\u{10000}', '😀', '�', 'ﬀ', '10', '2', 'a1', 'a😀', 'a￿', 'é', 'é', 'ß', 'ss'];
  out.cmpStr = [];
  for (const a of cmpS) for (const b of cmpS) out.cmpStr.push({ a, b, out: H.cmpStr(a, b) });
  for (const a of units) for (const b of units) out.cmpStr.push({ a: fromU(a), b: fromU(b), out: H.cmpStr(fromU(a), fromU(b)) });

  // --- Zeit
  const yrs = [];
  for (let y = 2018; y <= 2034; y++) yrs.push(y);
  const instants = [];
  for (const y of yrs) {
    const ls = (m) => { const d = new Date(Date.UTC(y, m + 1, 0)); return d.getUTCDate() - d.getUTCDay(); };
    for (const t of [Date.UTC(y, 2, ls(2), 1), Date.UTC(y, 9, ls(9), 1)]) for (const d of [-3600000, -1000, -1, 0, 1, 1000, 3600000]) instants.push(t + d);
    instants.push(Date.UTC(y, 0, 1), Date.UTC(y, 0, 1) - 1, Date.UTC(y, 11, 31, 23, 59, 59, 999), Date.UTC(y, 5, 21, 12));
  }
  instants.push(-1, 0, 1, Date.UTC(1969, 11, 31, 23), Date.UTC(1900, 5, 1), Date.UTC(2100, 2, 28), Date.UTC(2100, 2, 29, 1), Date.UTC(2038, 0, 19, 3, 14, 8));
  for (let i = 0; i < 400; i++) instants.push(Date.UTC(2025, 0, 1) + Math.floor(r.f() * 5 * 365 * 864e5));
  out.offsetMs = instants.map((t) => ({ in: t, out: T.offsetMs(t) }));
  out.broadcastDayOf = instants.map((t) => ({ in: t, out: T.broadcastDayOf(t) }));
  out.formatClock = instants.map((t) => ({ in: t, out: T.formatClock(t) }));
  // Sendetagsgrenzen um 05:00 lokal
  const dayEdges = [];
  for (let D = T.dayOfDate(2026, 0, 1); D <= T.dayOfDate(2028, 11, 31); D++) {
    const b = T.dayBounds(D);
    dayEdges.push(b.start - 1, b.start, b.start + 1, b.end - 1);
  }
  out.broadcastDayOfEdges = dayEdges.filter((_, i) => i % 3 === 0 || true).map((t) => ({ in: t, out: T.broadcastDayOf(t) }));
  const mins = [0, 1, 59, 60, 119, 120, 121, 150, 179, 180, 181, 239, 240, 299, 300, 301, 600, 1215, 1320, 1380, 1439, 1440, 1441, 1500, 1740, 2000, 2879, -1, -60, -120];
  const Ds = [];
  for (let D = T.dayOfDate(2026, 2, 25); D <= T.dayOfDate(2026, 3, 2); D++) Ds.push(D);
  for (let D = T.dayOfDate(2026, 9, 22); D <= T.dayOfDate(2026, 9, 28); D++) Ds.push(D);
  for (let D = T.dayOfDate(2027, 2, 25); D <= T.dayOfDate(2027, 3, 2); D++) Ds.push(D);
  for (let D = T.dayOfDate(2027, 9, 28); D <= T.dayOfDate(2027, 10, 2); D++) Ds.push(D);
  Ds.push(T.dayOfDate(1970, 0, 1), T.dayOfDate(2030, 5, 1), T.dayOfDate(2100, 2, 28));
  out.wallToUtc = [];
  for (const D of Ds) for (const m of mins) out.wallToUtc.push({ D, min: m, out: T.wallToUtc(D, m) });
  out.dayBounds = [];
  for (let D = T.dayOfDate(2025, 11, 25); D <= T.dayOfDate(2028, 11, 31); D++) out.dayBounds.push({ D, out: T.dayBounds(D) });
  out.formatDay = [];
  for (let D = T.dayOfDate(2026, 0, 1) - 400; D <= T.dayOfDate(2028, 11, 31); D += (D % 5 ? 1 : 1)) out.formatDay.push({ D, out: T.formatDay(D) });
  out.formatDay.push({ D: 0, out: T.formatDay(0) }, { D: -1, out: T.formatDay(-1) }, { D: -4, out: T.formatDay(-4) }, { D: -800, out: T.formatDay(-800) }, { D: 47482, out: T.formatDay(47482) });
  out.week = [];
  for (let D = -10; D <= 20; D++) out.week.push({ D, weekOf: T.weekOf(D), admit: null });
  for (let D = T.dayOfDate(2026, 5, 1); D <= T.dayOfDate(2027, 11, 31); D += 1) out.week.push({ D, weekOf: T.weekOf(D), weekStartOfWeek: T.weekStart(T.weekOf(D)) });
  out.weekStart = [];
  for (let W = -3; W <= 3000; W++) out.weekStart.push({ W, out: T.weekStart(W) });
  out.admitDay = instants.map((t) => ({ in: t, out: T.admitDay(t) }));
  const dod = [];
  for (const y of [1970, 1999, 2000, 2024, 2026, 2027, 2028, 2100]) for (const m of [0, 1, 2, 5, 9, 10, 11]) for (const d of [1, 15, 28, 29, 30, 31]) dod.push([y, m, d]);
  dod.push([2026, 12, 1], [2026, 0, 0], [2026, -1, 1], [2026, 10, 32], [2026, 1, 31], [70, 0, 1], [99, 11, 31], [0, 0, 1], [100, 0, 1], [2026, 10, 1.5]);
  out.dayOfDate = dod.map(([y, m, d]) => ({ y, m, d, out: T.dayOfDate(y, m, d) }));

  // --- parseDate / ticksToMs
  const pd = [
    '2026-11-01', '2026-11-01T00:00:00Z', '2026-11-01T10:20:30', '2026-11-01T10:20:30Z', '2026-11-01T10:20:30.5', '2026-11-01T10:20:30.123Z',
    '2026-11-01T10:20:30.1234567Z', '2026-11-01T10:20:30.0000000Z', '2026-11-01T10:20:30.9999999Z', '2026-11-01T10:20:30.123456789',
    '2024-05-01T23:59:59.9999999Z', '2026-11-01T10:20:30+02:00', '2026-11-01T10:20:30-05:30', '2026-11-01T10:20:30.250+01:00', '2026-11-01T10:20', '2026-11-01T10:20Z',
    ' 2026-11-01 ', '\t2026-11-01T01:02:03Z\n', '2026-11', '2026', '2026-11-01T24:00:00Z', '2026-11-01T24:00:01Z', '2026-13-01', '2024-02-29', '2024-02-29T12:00:00Z',
    '+002026-11-01T00:00:00Z', '-000001-01-01T00:00:00Z', '0000-01-01', '0001-01-01T00:00:00Z', '1969-12-31T23:59:59.999Z', '1970-01-01T00:00:00Z', '1970-01-01T00:00:00.001Z', '2100-12-31T23:59:59Z', '275760-09-13T00:00:00Z',
    '2026-11-01T10:20:30.Z', '2026-11-01T10:20:30.', 'T10:20:30', '', '   ', 'abc', 'null', 'NaN', '2026-11-01T', '2026-11-01Tfoo', '20261101',
    '2026-11-01T10:20:30,5Z', '2026-11-01T10:20:30.12Z', '2026-11-01T10:20:30.1Z',
  ];
  // V8-Sonderfälle (Legacy-Parser, teils lokalzeitabhängig): NICHT bindend für den Rust-Port, nur Dokumentation des V8-Verhaltens unter TZ=UTC
  const pdLegacy = ['2026-02-30', '2026-02-29', '2026/11/01', '2026-1-1', 'Sun, 01 Nov 2026 10:00:00 GMT', '2026-11-01T10:20:30z', '2026-11-01t10:20:30Z'];
  out.parseDateLegacy = pdLegacy.map((x) => ({ in: x, out: T.parseDate(x) }));
  const pdObjs = [0, 1, -1, 1790000000000, 1.5, NaN, Infinity, -Infinity, true, false, null, undefined, {}, [], ['2026-11-01'], 8.64e15, 8.64e15 + 1, -0, '1790000000000', 'Z'];
  out.parseDate = pd.concat(pdObjs).map((x) => ({ in: x, out: T.parseDate(x) }));
  const tk = [0, 1, 4999, 5000, 5001, 9999, 10000, 15000, 25000, 35000, 14999, -4999, -5000, -5001, -15000, -25000, -35000, 1e10, 36e9, 5.4e11, 1.8e12, 123456789012, 9007199254740991, 1e21, 1e-3, 0.5,
    '0', '5000', '3000000000', '  42  ', '1e4', '0x10', '', ' ', 'abc', 'Infinity', null, undefined, NaN, Infinity, -Infinity, true, false, [], [5], {}, '-5000', '-15000'];
  out.ticksToMs = tk.map((x) => ({ in: x, out: T.ticksToMs(x) }));

  // --- Altersfreigabe
  const rat = ['G', 'g', 'U', 'TV-Y', 'TV-G', 'PG', 'TV-PG', 'TV-Y7', 'TV-Y7-FV', 'R', 'TV-MA', 'NC-17', 'X', 'XXX', 'AO', 'R18', 'NR', 'UNRATED', 'Unrated', '', ' ', null, undefined,
    'FSK 0', 'FSK 6', 'FSK 12', 'FSK 16', 'FSK 18', 'FSK-12', 'FSK12', 'ab 12', 'ab 16', 'ab 18', 'DE-0', 'DE-6', 'DE-12', 'DE-16', 'DE-18', '6', '12', '16', '18', '0', '7', '9', '11', '13', '14', '15', '17', '19', '21', '99', '100', '123',
    '12+', '16+', '18+', '6+', '12A', '15A', '18A', '12+A', 'PG-13', 'PG-12', 'TV-14', 'TV-13', 'PEGI 18', 'PEGI 7', 'M', 'MA15+', 'MA 15+', 'R12', 'B15', 'Freigegeben ab 12 Jahren', 'ab 12 Jahren', 'FSK 12 ', ' fsk 16', '٣', '１２', 5, 12, 18, 0, true];
  out.ratingAge = rat.map((x) => ({ in: x, out: R.ratingAge(x) }));
  const ages = [null, undefined, 0, 6, 12, 16, 18];
  out.maxAge = [];
  for (const a of ages) for (const b of ages) out.maxAge.push({ a, b, out: R.maxAge(a, b) });

  // --- Katalog-Helfer
  const gen = ['Aktion', ' Aktion ', 'AKTION', 'aktion', 'Komödie', 'KOMÖDIE', 'Straße', 'STRASSE', 'strauß', 'STRAUSS', 'İstanbul', 'i̇stanbul', 'I', 'ı', 'İ', 'ΑΣ', 'ας', 'ΑΣΑ', 'Σ', 'ΟΔΟΣ ', 'Σ Σ', 'Café', 'Café', 'CAFÉ', 'ẞ', 'ǅ', 'ǆ', 'Ǆ', 'ﬁlm', 'FİLM', 'Å', 'Å', 'Å', 'Ω', 'Ω', '', ' ', null, undefined, 5, 'Sci-Fi', 'sci-fi', '日本', '😀 Spaß', 'Ⅷ', 'ⅷ', 'İ̇', 'ßß', 'ꭰ', 'Ꭰ', 'Ა', 'ᲐᲑ', 'Ꙋ', 'ς', 'ͅ', 'ΑΣͅ', 'ΑΣ.', 'ΑΣ.Α', 'A.Σ', 'Σ.', '.Σ', 'ΑΙΣ', 'ΑΙ Σ', 'ΆΣ'];
  out.normGenre = gen.map((x) => ({ in: x, out: C.normGenre(x) }));
  const meds = [[], [5], [1, 2], [1, 2, 3], [1, 2, 3, 4], [10, 20, 30, 40, 50], [7, 7, 7], [0, 0, 1]];
  out.lowerMedian = meds.map((a) => ({ in: a, out: C.lowerMedian(a) }));

  // --- Pool
  out.streamOfAge = [null, undefined, 0, 6, 12, 13, 16, 18].map((a) => ({ in: a, out: P.streamOfAge(a) }));
  out.bucketCount = [];
  for (const cap of [1, 36e5, 4 * 36e5, 17 * 36e5, 1e9]) for (const pm of [0, 1, cap - 1, cap, 2 * cap - 1, 2 * cap, 3 * cap, 4 * cap, 5 * cap, 1000 * cap + 3, 4096 * cap, 8192 * cap, 1e15]) out.bucketCount.push({ poolMs: pm, capMs: cap, out: P.bucketCount(pm, cap) });
  out.capMs = P.CAP_MS;
  out.entries = [];
  for (const n of [0, 1, 2, 3, 7, 20, 50, 300]) {
    for (const key of (n >= 50 ? ['kino', 'premiere'] : ['kino', 'premiere', 'themeday:boxset', 'genre:aktion'])) {
      const items = [];
      for (let i = 0; i < n; i++) items.push({ id: i % 11 === 0 ? 'é' + r.hex(3) : r.hex(r.chance(0.2) ? 4 : 16), durMs: r.range(5, 300) * 6e4 });
      const seen = new Set();
      const uniq = items.filter((x) => (seen.has(x.id) ? false : (seen.add(x.id), true)));
      const sorted = P.makeEntries(key, uniq);
      const cases = [];
      for (const Cc of [1, 2, 8, 32]) {
        for (const D of [0, 1, 2, 3, 7, 8, 9, 31, 32, 33, 20000, 20001, 20340, 29999, 1073741823, 1073741824, 2147483647]) {
          const ids = (l) => l.map((e) => e.id);
          cases.push({ C: Cc, D, bucket: ids(P.bucket(sorted, Cc, D)), rotation: ids(P.rotation(sorted, Cc, D)), borrow: { home: ids(P.rotationWithBorrow(sorted, Cc, D).home), borrow: ids(P.rotationWithBorrow(sorted, Cc, D).borrow) } });
        }
      }
      out.entries.push({ key, items: uniq, sorted: sorted.map((e) => ({ id: e.id, h: e.h, br: e.br, ms: e.ms })), cases });
    }
  }
  // makeEntries mit eigenem msOf (Länge der Id)
  const it2 = ['a', 'bb', 'ccc', 'dddd', 'e', 'ff'].map((id) => ({ id, durMs: 60000 }));
  out.entriesMsOf = { items: it2, msOf: 'id.length*1000', out: P.makeEntries('x', it2, (it) => it.id.length * 1000).map((e) => ({ id: e.id, h: e.h, br: e.br, ms: e.ms })) };

  // --- Füllbaustein
  out.slotOf = [0, 1, 299999, 300000, 300001, 1799999, 1800000, 2700001, 36e5, 5 * 36e5, 1e-3, -1, 7200000].map((d) => ({ in: d, out: F.slotOf(d) }));
  out.ageAllowed = [];
  for (const D of [T.dayOfDate(2026, 10, 3), T.dayOfDate(2026, 9, 24), T.dayOfDate(2027, 2, 27)]) {
    const b = T.dayBounds(D);
    for (const age of [null, undefined, 0, 6, 12, 16, 18]) for (const st of [b.start, b.a2015, b.a2200 - 1, b.a2200, b.a2300 - 1, b.a2300, b.end - 1]) out.ageAllowed.push({ D, age, start: st, out: F.ageAllowed(age, st, b) });
  }
  out.packSubset = [];
  for (let i = 0; i < 150; i++) {
    const n = r.range(0, i < 20 ? 6 : 45);
    const sizes = Array.from({ length: n }, () => r.range(1, i % 3 ? 14 : 60));
    const cap = [0, 1, 5, 24, 48, 100, 200, 1200, 1500, -3][r.int(10)];
    out.packSubset.push({ sizes, cap, out: F.packSubset(sizes, cap) });
  }
  out.packSubset.push({ sizes: [3, 3, 3, 3], cap: 6, out: F.packSubset([3, 3, 3, 3], 6) }, { sizes: [5, 5, 10], cap: 10, out: F.packSubset([5, 5, 10], 10) }, { sizes: [10, 5, 5], cap: 10, out: F.packSubset([10, 5, 5], 10) });
  out.closedownDay = [T.dayOfDate(2026, 10, 1), T.dayOfDate(2026, 9, 24), T.dayOfDate(2027, 2, 27)].flatMap((D) => [{ D, reason: undefined, out: F.closedownDay(D) }, { D, reason: 'empty', out: F.closedownDay(D, 'empty') }, { D, reason: 'end', out: F.closedownDay(D, 'end') }]);

  // --- Senderbelegung (tvdata/config.js, nur Format-Parser; plan.js hat einen eigenen, siehe channels-Vektoren)
  const cfgs = ['5=Action;6=Komödie', ' 5 = "Action" ; 6=\'Komödie\' ', '4=x;5=a;16=b;17=c', '5=;6= ;7=x', 'a=b;5', '5=a=b', '', '  ', ';;;', '5=A;5=B', '"5=A"', "'5=A;6=B'", '05=A', '5.0=A', '5e0=A', '0x5=A', '+5=A', '5 =A', null, undefined, 7, true];
  out.parseChannelConfigFmt = cfgs.map((x) => ({ in: x, out: cfgMod.parseChannelConfig(x) }));
  for (const o of [{ 5: 'A', 6: ' B ', 7: 3, 8: null, 4: 'x', 17: 'y', x: 'z', 16: '"Q"' }, {}, { 5: '' }, [], ['a', 'b']]) out.parseChannelConfigFmt.push({ in: o, out: cfgMod.parseChannelConfig(o) });
  return out;
}

// ---------------------------------------------------------------------------------------------- Füllbaustein-Szenarien

function builderScenarios() {
  const r = rng(202);
  const scenarios = [];
  for (let sc = 0; sc < 24; sc++) {
    const D = [T.dayOfDate(2026, 10, 3), T.dayOfDate(2026, 9, 24), T.dayOfDate(2027, 2, 27), T.dayOfDate(2027, 9, 30), T.dayOfDate(2026, 10, 15)][sc % 5];
    const nShort = sc % 6 === 5 ? 0 : r.range(1, 40);
    const shorts = [];
    for (let i = 0; i < nShort; i++) shorts.push({ id: 's' + r.hex(6), durMs: r.range(5, 50) * 6e4, age: r.pick([null, null, 0, 6, 12, 16, 18]), seriesId: r.chance(0.3) ? 'ser' + r.int(5) : undefined });
    const key = r.pick(['5', '1', '17', undefined, 'x']);
    const usedInit = r.chance(0.3) && shorts.length ? [shorts[0].id, 'zz'] : [];
    const b = T.dayBounds(D);
    const builder = F.createDayBuilder({ D, used: new Set(usedInit), shorts, key });
    const ops = [];
    const log = [];
    const nOps = r.range(2, 14);
    for (let i = 0; i < nOps; i++) {
      const kind = r.weighted([['place', 6], ['placeBlock', 3], ['canPlace', 2], ['fillUntil', 3], ['closeRest', 0.4]]);
      if (kind === 'place' || kind === 'canPlace') {
        const piece = { itemId: r.chance(0.1) && i ? 'dup' : 'p' + r.hex(5), durMs: r.range(2, 200) * 6e4 + (r.chance(0.2) ? r.range(1, 59999) : 0), age: r.pick([null, 0, 12, 16, 18]) };
        if (r.chance(0.3)) piece.seriesId = 'ser' + r.int(3);
        if (r.chance(0.2)) piece.rerun = true;
        if (r.chance(0.1)) piece.premiere = r.pick(['film', 'episode']);
        if (r.chance(0.1)) piece.theme = 'Thema';
        if (r.chance(0.1)) piece.label = 'Etikett';
        const opts = r.chance(0.4) ? { limit: b.start + r.range(1, 24) * 36e5 } : r.chance(0.2) ? { type: 'fill' } : undefined;
        ops.push({ op: kind, piece, opts });
        log.push(builder[kind === 'place' ? 'place' : 'canPlace'](piece, opts));
      } else if (kind === 'placeBlock') {
        const pieces = Array.from({ length: r.range(0, 4) }, () => ({ itemId: r.chance(0.1) ? 'dup' : 'b' + r.hex(5), durMs: r.range(10, 60) * 6e4, age: r.pick([null, 12, 16]) }));
        const opts = r.chance(0.3) ? { limit: b.start + r.range(1, 24) * 36e5, type: 'program' } : undefined;
        ops.push({ op: 'placeBlock', pieces, opts });
        log.push(builder.placeBlock(pieces, opts));
      } else if (kind === 'fillUntil') {
        const until = b.start + Math.floor(r.range(0, 26 * 12) * 5 * 6e4);
        ops.push({ op: 'fillUntil', until });
        log.push(null);
        builder.fillUntil(until);
      } else {
        ops.push({ op: 'closeRest', reason: r.pick([undefined, 'end', 'x']) });
        log.push(null);
        builder.closeRest(ops[ops.length - 1].reason);
      }
      log[log.length - 1] = [log[log.length - 1], builder.cursor];
    }
    const entries = builder.finish();
    scenarios.push({ D, key, shorts, usedInit, ops, results: log, finish: entries, usedAfter: [...builder.used] });
  }
  // fillSegment-Szenarien (strict und nicht strict, Teilsummen-DP)
  const segs = [];
  for (let sc = 0; sc < 30; sc++) {
    const D = T.dayOfDate(2026, 10, 3);
    const b = T.dayBounds(D);
    const shorts = Array.from({ length: 12 }, (_, i) => ({ id: 'q' + i + r.hex(4), durMs: r.range(5, 50) * 6e4, age: null }));
    const builder = F.createDayBuilder({ D, shorts, key: '3' });
    const cands = Array.from({ length: r.range(0, 60) }, (_, i) => {
      const n = r.chance(0.7) ? 1 : r.range(2, 3);
      const pieces = Array.from({ length: n }, (_, j) => ({ itemId: 'c' + i + '_' + j, durMs: r.range(20, 150) * 6e4, age: r.pick([null, null, 12, 16, 18]) }));
      return { id: 'c' + i, pieces, size: pieces.reduce((x, p) => x + F.slotOf(p.durMs), 0) };
    });
    const strict = sc % 3 !== 0;
    const limit = b.start + r.pick([3, 6, 9, 15, 15.25, 17, 19.5, 24]) * 36e5;
    if (sc % 4 === 1) builder.fillUntil(b.start + 2 * 36e5);
    F.fillSegment(builder, cands, { limit, strict });
    segs.push({ D, shorts, cands, strict, limit, pre: sc % 4 === 1 ? { fillUntil: b.start + 2 * 36e5 } : null, finish: builder.finish() });
  }
  return { scenarios, segs };
}

// ---------------------------------------------------------------------------------------------- (b) synthetische Rohkataloge

const GENRES_NORMAL = ['Aktion', 'Komödie', 'Drama', 'Sci-Fi', 'Krimi', 'Horror', 'Familie', 'Doku', 'Fantasy', 'Anime', 'Western', 'Thriller', 'Abenteuer', 'Strauß', 'İstanbul', 'ΑΣ', 'Café'];
const RATINGS = [['FSK 0', 10], ['FSK 6', 15], ['FSK 12', 25], ['FSK 16', 22], ['FSK 18', 8], ['PG-13', 3], ['TV-MA', 3], ['G', 2], ['NR', 3], ['', 2], [null, 7]];
const iso7 = (t) => new Date(t).toISOString().replace('Z', '0000Z');
const iso3 = (t) => new Date(t).toISOString();
const isoDate = (t) => new Date(t).toISOString().slice(0, 10);

function fmtDate(r, t) {
  if (t == null) return null;
  const x = r.f();
  if (x < 0.68) return iso7(t);
  if (x < 0.88) return iso3(t);
  if (x < 0.94) return isoDate(t);
  if (x < 0.97) return iso3(t).slice(0, 19);
  return iso7(t);
}

function compact(it, keys) {
  const out = {};
  for (const k of keys) if (it[k] !== undefined && it[k] !== null) out[k] = it[k];
  if (out.ImageTags) {
    const t = {};
    if (out.ImageTags.Thumb) t.Thumb = out.ImageTags.Thumb;
    if (out.ImageTags.Primary) t.Primary = out.ImageTags.Primary;
    out.ImageTags = t;
  }
  if (Array.isArray(out.BackdropImageTags)) out.BackdropImageTags = out.BackdropImageTags.slice(0, 1);
  return out;
}

const T_OLD0 = Date.UTC(2023, 0, 1);
const T_OLD1 = Date.UTC(2026, 9, 31, 23);
const T_NEW0 = Date.UTC(2026, 10, 1, 4);
const T_NEW1 = Date.UTC(2027, 3, 30);

function genAddedAt(r, newShare) {
  if (r.chance(newShare)) {
    return r.chance(0.5) ? T_NEW0 + Math.floor(r.f() * 20 * 864e5) : T_NEW0 + Math.floor(r.f() * (T_NEW1 - T_NEW0));
  }
  return T_OLD0 + Math.floor(r.f() * (T_OLD1 - T_OLD0));
}

function genGenres(r, pool, maxN) {
  const n = r.weighted([[1, 5], [2, 4], [3, 2], [0, 0.3]]);
  const out = [];
  for (let i = 0; i < Math.min(n, maxN); i++) {
    // Zipf-artig: niedrige Indizes häufiger
    const idx = Math.min(pool.length - 1, Math.floor(Math.pow(r.f(), 1.6) * pool.length));
    let g = pool[idx];
    const x = r.f();
    if (x < 0.06) g = g.toUpperCase();
    else if (x < 0.1) g = g.toLowerCase();
    else if (x < 0.13) g = '  ' + g + ' ';
    else if (x < 0.15 && g === 'Café') g = 'Café';
    out.push(g);
  }
  if (out.length && r.chance(0.05)) out.push(out[0]);
  if (r.chance(0.02)) out.push(null);
  return out;
}

/** opts: { seed, nMovies, nSeries, epMin, epMax, nBoxsets, genres, newShare, edge } */
function genRaw(o) {
  const r = rng(o.seed);
  const usedIds = new Set();
  const newId = (len = 16) => { for (;;) { const id = r.hex(len); if (!usedIds.has(id)) { usedIds.add(id); return id; } } };
  const movies = [];
  for (let i = 0; i < o.nMovies; i++) {
    const kind = r.weighted([['std', 70], ['long', 10], ['short', 8], ['bad', 4], ['none', 3], ['mid', 5]]);
    let min;
    if (kind === 'std') min = r.range(80, 140);
    else if (kind === 'long') min = r.range(150, 200);
    else if (kind === 'short') min = r.range(6, 49);
    else if (kind === 'bad') min = r.pick([1, 2, 4, 301, 360, 700]);
    else min = r.range(51, 79);
    const ms = min * 6e4 + (r.chance(0.5) ? r.int(60000) : 0);
    let ticks = kind === 'none' ? (r.chance(0.5) ? 0 : undefined) : ms * 1e4;
    if (typeof ticks === 'number' && ticks > 0 && r.chance(0.05)) ticks = String(ticks);
    const added = genAddedAt(r, o.newShare);
    const yr = r.f();
    const prem = r.chance(0.6) ? Date.UTC(r.range(1950, 2026), r.int(12), r.range(1, 28)) : null;
    const m = {
      Id: newId(), Name: 'Film ' + String(i + 1).padStart(4, '0'), RunTimeTicks: ticks, Genres: genGenres(r, o.genres, 3),
      OfficialRating: r.weighted(RATINGS), ProductionYear: yr < 0.8 ? r.range(1950, 2026) : yr < 0.9 ? undefined : yr < 0.93 ? 0 : yr < 0.96 ? 1800 : String(r.range(1950, 2026)),
      PremiereDate: prem == null ? undefined : fmtDate(r, prem), DateCreated: r.chance(0.02) ? undefined : fmtDate(r, added),
      LocalTrailerCount: r.chance(0.4) ? r.int(3) : undefined, ImageTags: r.chance(0.8) ? { Primary: r.hex(32), Thumb: r.chance(0.3) ? r.hex(32) : undefined, Logo: r.hex(8) } : undefined,
      BackdropImageTags: r.chance(0.5) ? [r.hex(32), r.hex(32)] : [],
    };
    movies.push(compact(m, KEEP.movies));
  }
  const series = [];
  const episodes = [];
  for (let s = 0; s < o.nSeries; s++) {
    const sid = newId();
    const sAdded = genAddedAt(r, o.newShare * 0.6);
    const sName = 'Serie ' + String(s + 1).padStart(3, '0');
    series.push(compact({
      Id: sid, Name: sName, Genres: genGenres(r, o.genres, 2), OfficialRating: r.weighted(RATINGS), ProductionYear: r.chance(0.85) ? r.range(1980, 2026) : undefined,
      DateCreated: fmtDate(r, sAdded), ImageTags: { Primary: r.hex(32) }, BackdropImageTags: [r.hex(32)],
    }, KEEP.series));
    const base = r.pick([22, 24, 42, 45, 50, 58]);
    const seasons = r.range(1, 4);
    const nEps = r.range(o.epMin, o.epMax);
    const epsPer = Math.max(1, Math.ceil(nEps / seasons));
    let n = 0;
    for (let se = 1; se <= seasons; se++) {
      let batchAt = sAdded + r.int(3) * 36e5;
      for (let e = 1; e <= epsPer && n < nEps; e++, n++) {
        if (r.chance(0.25)) batchAt += r.range(1, 40) * 864e5 * (r.chance(0.3) ? 1 : 0);
        const dm = r.weighted([['n', 80], ['miss', 6], ['zero', 3], ['bad', 3], ['var', 8]]);
        let ticks;
        if (dm === 'n') ticks = (base + r.range(-3, 3)) * 6e4 * 1e4 + r.int(60000) * 1e4;
        else if (dm === 'var') ticks = r.range(8, 90) * 6e4 * 1e4;
        else if (dm === 'zero') ticks = 0;
        else if (dm === 'bad') ticks = r.pick([1, 2, 3, 310, 400]) * 6e4 * 1e4;
        const ep = {
          Id: newId(), Name: sName + ' S' + se + 'E' + e, SeriesId: sid, SeriesName: r.chance(0.9) ? sName : undefined,
          ParentIndexNumber: r.chance(0.03) ? undefined : se, IndexNumber: r.chance(0.03) ? undefined : e, RunTimeTicks: ticks,
          PremiereDate: r.chance(0.7) ? fmtDate(r, Date.UTC(2015 + se, r.int(12), r.range(1, 28))) : undefined,
          DateCreated: r.chance(0.02) ? undefined : fmtDate(r, batchAt + r.int(3000) * 1000), OfficialRating: r.chance(0.15) ? r.weighted(RATINGS) : undefined,
        };
        episodes.push(compact(ep, KEEP.episodes));
      }
    }
    if (r.chance(0.08)) { // Extras (Staffel 0)
      episodes.push(compact({ Id: newId(), Name: sName + ' Extra', SeriesId: sid, ParentIndexNumber: 0, IndexNumber: 1, RunTimeTicks: 20 * 6e4 * 1e4, DateCreated: fmtDate(r, sAdded) }, KEEP.episodes));
    }
  }
  // Folgen ohne Serie (verwaist)
  for (let i = 0; i < Math.max(2, Math.floor(o.nSeries / 10)); i++) {
    episodes.push(compact({ Id: newId(), Name: 'Verwaist ' + i, SeriesId: newId(), ParentIndexNumber: 1, IndexNumber: 1, RunTimeTicks: 30 * 6e4 * 1e4, DateCreated: iso7(T_OLD0) }, KEEP.episodes));
  }
  const boxsets = [];
  for (let b = 0; b < o.nBoxsets; b++) {
    boxsets.push(compact({ Id: newId(), Name: 'Reihe ' + String(b + 1).padStart(2, '0'), ChildCount: r.weighted([[r.range(2, 6), 8], [1, 1], [0, 1], [undefined, 1]]) }, KEEP.boxsets));
  }
  // Doppelte Ids mit abweichendem Inhalt (dedupe-Gleichstand per JSON.stringify, Schlüsselreihenfolge = KEEP)
  const dupN = Math.max(1, Math.floor(o.nMovies / 60));
  for (let i = 0; i < dupN; i++) {
    const src = movies[r.int(movies.length)];
    movies.push({ ...src, Name: src.Name + (r.chance(0.5) ? ' (Kopie)' : ''), ...(r.chance(0.5) ? { RunTimeTicks: 99 * 6e4 * 1e4 } : {}) });
    if (r.chance(0.5)) movies.push({ ...src }); // exakt gleich
  }
  if (series.length) { const s0 = series[0]; series.push({ ...s0, Name: 'Serie 000 Kopie' }); }
  if (episodes.length) { const e0 = episodes[0]; episodes.push({ ...e0, RunTimeTicks: 77 * 6e4 * 1e4 }); }
  if (boxsets.length) boxsets.push({ ...boxsets[0], Name: 'Reihe Kopie' });
  // fehlende/ungültige Einträge
  movies.push(null, { Name: 'ohne Id', RunTimeTicks: 90 * 6e4 * 1e4 }, { Id: '', Name: 'leere Id', RunTimeTicks: 90 * 6e4 * 1e4 });
  r.shuffle(movies); r.shuffle(series); r.shuffle(episodes); r.shuffle(boxsets);
  return { movies, series, episodes, boxsets, loadedAt: 1790000000000 };
}

function genEdge() {
  const r = rng(909);
  const base = genRaw({ seed: 909, nMovies: 70, nSeries: 12, epMin: 3, epMax: 9, nBoxsets: 5, genres: GENRES_NORMAL.slice(0, 13), newShare: 0.35 });
  const mk = (keys, o) => compact(o, keys);
  const mv = (Id, extra) => mk(KEEP.movies, { Id, Name: 'Film Rand ' + String(Id), RunTimeTicks: 100 * 6e4 * 1e4, Genres: ['Drama'], OfficialRating: 'FSK 12', ProductionYear: 2001, DateCreated: iso7(Date.UTC(2025, 5, 1)), ...extra });
  const extraMovies = [
    mv('é1'), mv('Z9'), mv('a😀'), mv('a￿'), mv('10'), mv('2'), mv('A1'), mv('a1'), mv(0, { Name: 'Id null als Zahl' }), mv(5), mv('5', { Name: 'Id 5 als String (Duplikat zu Zahl 5)' }),
    mv('u1', { Genres: ['Straße', 'STRASSE', 'strauße', 'İstanbul', 'i̇stanbul', 'ΑΣ', 'ας', 'Café', 'Café', 'CAFÉ'] }),
    mv('u2', { Genres: 'Drama', RunTimeTicks: '6000000000' }), mv('u3', { Genres: [5, true, {}, ' ', ''], RunTimeTicks: 3000000000 }),
    mv('u4', { PremiereDate: 'kaputt', DateCreated: 'auch kaputt', ProductionYear: 'abc' }), mv('u5', { RunTimeTicks: 3000000000, ProductionYear: undefined, PremiereDate: '1999-12-31' }),
    mv('u6', { RunTimeTicks: 3e9 - 1 }), mv('u7', { RunTimeTicks: 3e9 + 5000 }), mv('u8', { RunTimeTicks: 18e10 }), mv('u9', { RunTimeTicks: 18e10 + 1e4 }), mv('u10', { RunTimeTicks: 3e9 + 4999 }),
    mv('u11', { OfficialRating: 'PEGI 18' }), mv('u12', { OfficialRating: 'ab 16' }), mv('u13', { OfficialRating: '12+' }), mv('u14', { ProductionYear: 2999 }), mv('u15', { ProductionYear: 3000 }), mv('u16', { ProductionYear: 1801 }),
    mv('u17', { LocalTrailerCount: '2' }), mv('u18', { LocalTrailerCount: 'x' }), mv('u19', { Name: null }), mv('u20', { DateCreated: '2026-11-03T01:02:03.1234567Z' }), mv('u21', { DateCreated: '2026-11-03T01:02:03+02:00' }),
  ];
  base.movies.push(...extraMovies);
  // Serie mit numerischen Ids, Folge mit gleicher Id wie ein Film (Map-Kollision), Serie ohne planbare Folgen, Serie ohne Folgen-Laufzeit
  const eid = 'e' + r.hex(6);
  base.series.push(mk(KEEP.series, { Id: 7, Name: 'Serie Zahl', Genres: ['Drama'], OfficialRating: 'FSK 16', ProductionYear: 2010, DateCreated: iso7(Date.UTC(2025, 1, 1)) }));
  for (let i = 1; i <= 4; i++) base.episodes.push(mk(KEEP.episodes, { Id: 'z' + i, Name: 'Zahl-Folge ' + i, SeriesId: 7, ParentIndexNumber: 1, IndexNumber: i, RunTimeTicks: 25 * 6e4 * 1e4, DateCreated: iso7(Date.UTC(2025, 1, 2 + i)) }));
  base.series.push(mk(KEEP.series, { Id: 'leer', Name: 'Serie leer', Genres: ['Drama'], DateCreated: iso7(Date.UTC(2025, 1, 1)) }));
  base.series.push(mk(KEEP.series, { Id: 'ohnelz', Name: 'Serie ohne Laufzeit', Genres: ['Krimi'], DateCreated: iso7(Date.UTC(2025, 1, 1)) }));
  base.episodes.push(mk(KEEP.episodes, { Id: 'ol1', Name: 'ohne Laufzeit', SeriesId: 'ohnelz', ParentIndexNumber: 1, IndexNumber: 1 }));
  base.series.push(mk(KEEP.series, { Id: 'xt', Name: 'Serie Tie', Genres: ['Krimi', 'Drama'], DateCreated: iso7(Date.UTC(2025, 1, 1)) }));
  const tie = (n, o) => mk(KEEP.episodes, { Id: 'tie' + n, Name: 'Gleichstand ' + n, SeriesId: 'xt', ParentIndexNumber: 1, IndexNumber: 1, RunTimeTicks: 24 * 6e4 * 1e4, PremiereDate: '2020-01-01', DateCreated: iso7(Date.UTC(2025, 1, 5)), ...o });
  base.episodes.push(tie(1), tie(2), tie(3, { IndexNumber: undefined }), tie(4, { ParentIndexNumber: undefined }), tie(5, { PremiereDate: undefined }), tie(6, { ParentIndexNumber: 2, IndexNumber: 1 }), tie(7, { ParentIndexNumber: 0 }));
  base.episodes.push(mk(KEEP.episodes, { Id: 'é1', Name: 'Folge mit Id eines Films', SeriesId: 'xt', ParentIndexNumber: 3, IndexNumber: 1, RunTimeTicks: 24 * 6e4 * 1e4, DateCreated: iso7(Date.UTC(2025, 1, 6)) })); // Kollision mit Film é1
  base.episodes.push(mk(KEEP.episodes, { Id: eid, Name: 'Folge ohne Serienname', SeriesId: 'xt', ParentIndexNumber: 3, IndexNumber: 2, RunTimeTicks: '1500000000', DateCreated: iso7(Date.UTC(2025, 1, 6)) }));
  base.boxsets.push(mk(KEEP.boxsets, { Id: 'bx1', Name: 'Reihe Rand', ChildCount: '3' }), mk(KEEP.boxsets, { Id: 'bx2', Name: 'Reihe Rand 2', ChildCount: 'x' }));
  r.shuffle(base.movies); r.shuffle(base.series); r.shuffle(base.episodes); r.shuffle(base.boxsets);
  return base;
}

// ---------------------------------------------------------------------------------------------- Boxset-Inhalte (Mock)

function genBoxsetContents(raw, seed) {
  const r = rng(seed);
  const movieIds = raw.movies.filter((m) => m && m.Id != null && m.Id !== '').map((m) => String(m.Id));
  const epIds = raw.episodes.map((e) => String(e.Id));
  const out = {};
  for (const b of raw.boxsets) {
    if (!b || b.Id == null) continue;
    const id = String(b.Id);
    const x = r.f();
    if (x < 0.1) { out[id] = { $: 'throw' }; continue; }
    if (x < 0.18) { out[id] = null; continue; }
    const n = r.range(0, 8);
    const list = [];
    for (let i = 0; i < n; i++) list.push(r.chance(0.8) ? r.pick(movieIds) : r.chance(0.6) ? r.pick(epIds) : 'unbekannt' + r.hex(4));
    if (list.length && r.chance(0.3)) list.push(list[0]);
    out[id] = list;
  }
  return out;
}

// ---------------------------------------------------------------------------------------------- Plan-Fenster

const WINDOWS = {
  w0: { label: '14 Sendetage ab 2026-11-01', from: T.dayOfDate(2026, 10, 1), n: 14 },
  w1: { label: 'Zeitumstellung Herbst 2026 (25-h-Tag 24.10.)', from: T.dayOfDate(2026, 9, 23), n: 5 },
  w2: { label: 'Zeitumstellung Frühjahr 2027 (23-h-Tag 27.03.)', from: T.dayOfDate(2027, 2, 26), n: 5 },
  w3: { label: 'Zeitumstellung Herbst 2027 (25-h-Tag 30.10.)', from: T.dayOfDate(2027, 9, 29), n: 5 },
};

function stripFrozen(x) { return JSON.parse(JSON.stringify(enc(x))); }

async function runWindow(rawCat, boxContents, variant, win, { prepare = true, reverse = false } = {}) {
  const calls = [];
  const sched = index.createScheduler(index.normalizeCatalog(rawCat), {
    channelConfig: variant.config,
    getBoxSetItems: async (id) => {
      calls.push(id);
      const v = boxContents[id];
      if (v && v.$ === 'throw') throw new Error('Mock-Fehler');
      return v === undefined ? [] : v;
    },
  });
  const days = Array.from({ length: win.n }, (_, i) => win.from + i);
  if (prepare) await sched.prepare(days);
  const plans = {};
  const dayList = reverse ? days.slice().reverse() : days;
  const chList = reverse ? [18, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1] : [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18];
  const chans = {};
  for (const D of dayList) {
    chans[D] = sched.channels(D);
    for (const no of chList) { (plans[no] = plans[no] || {})[D] = sched.planDay(no, D); }
  }
  return { sched, days, plans, chans, calls };
}

async function main() {
  const meta = {
    generator: 'tools/golden/gen.mjs',
    webCommit: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: WEB }).toString().trim(),
    webSchedulerDirty: execFileSync('git', ['status', '--porcelain', '--', 'src/js/tv/schedule', 'src/js/features/tvdata'], { cwd: WEB }).toString().trim() !== '',
    node: process.version,
    v8: process.versions.v8,
    tz: 'UTC',
    keepVerifiedAgainstWeb: keepChecked,
    conventions: 'siehe README.md: Marker {"$": ...}',
    windows: Object.fromEntries(Object.entries(WINDOWS).map(([k, w]) => [k, { label: w.label, fromD: w.from, days: w.n }])),
    files: [],
  };

  const prim = primitives();
  const poolPart = { entries: prim.entries, entriesMsOf: prim.entriesMsOf, bucketCount: prim.bucketCount, capMs: prim.capMs, streamOfAge: prim.streamOfAge };
  for (const k of Object.keys(poolPart)) delete prim[k];
  writeJson('primitives.json', prim);
  writeJson('primitives-pool.json', poolPart);
  writeJson('builder.json', builderScenarios());

  const cats = {
    thin: { raw: genRaw({ seed: 11, nMovies: 24, nSeries: 3, epMin: 3, epMax: 7, nBoxsets: 2, genres: GENRES_NORMAL.slice(0, 5), newShare: 0.3 }), variants: [{ name: 'default', config: undefined }, { name: 'cfgstr', config: '5=komödie; 6 = Drama;12=Unbekannt' }] },
    normal: {
      raw: genRaw({ seed: 22, nMovies: 400, nSeries: 40, epMin: 10, epMax: 30, nBoxsets: 10, genres: GENRES_NORMAL, newShare: 0.3 }),
      variants: [
        { name: 'default', config: undefined },
        { name: 'cfgstr', config: ' 5 = aktion ; 6=KOMÖDIE;7=nonexistent;8=;x=drama;4=Drama;17=Drama;9=Drama;10=drama;16=Sci-Fi;11= Drama ;12=Strauß;13=STRASSE' },
        { name: 'cfgobj', config: { 5: 'Drama', 6: 'Krimi', 7: '  HORROR ', 8: 123, 20: 'Drama', x: 'Aktion', 9: null, 10: 'Krimi', 4: 'Doku' } },
        { name: 'noprepare', config: undefined, prepare: false },
      ],
    },
    large: { raw: genRaw({ seed: 33, nMovies: 2400, nSeries: 120, epMin: 15, epMax: 50, nBoxsets: 25, genres: GENRES_NORMAL, newShare: 0.25 }), variants: [{ name: 'default', config: undefined }, { name: 'cfgstr', config: '5=Drama;7=Doku;16=Western;15=Aktion', windows: ['w0'] }] },
    edge: { raw: genEdge(), variants: [{ name: 'default', config: undefined }, { name: 'cfgstr', config: '5=İSTANBUL;6=ΑΣ;7=strauß;8=Café;9=STRASSE;10=i̇stanbul', windows: ['w0', 'w2'] }] },
  };

  const nowFiles = [];
  for (const [cname, cat] of Object.entries(cats)) {
    const norm = index.normalizeCatalog(cat.raw);
    const boxContents = genBoxsetContents(cat.raw, 5000 + cname.length);
    // Rohkatalog + normalisiert
    writeJson(`catalog-${cname}.raw.json`, { name: cname, raw: cat.raw, boxsetContents: boxContents });
    const labelProbe = norm.genres.map((g) => g.norm).concat(['unbekannt', '', 'x']);
    writeJson(`catalog-${cname}.norm.json`, {
      name: cname,
      movies: norm.movies, series: norm.series,
      itemsOrder: [...norm.items.keys()].map((id) => [id, norm.items.get(id).kind]),
      boxsets: norm.boxsets, genres: norm.genres, loadedAt: norm.loadedAt,
      genreLabel: labelProbe.map((n) => ({ in: n, out: norm.genreLabel(n) })),
      genresApi: index.createScheduler(norm).genres(),
    });
    if (cname === 'normal') {
      // Eingabereihenfolge-Unabhängigkeit der Normalisierung (Selbstprüfung)
      const rr = rng(77);
      const shuffled = { ...cat.raw, movies: rr.shuffle(cat.raw.movies.slice()), series: rr.shuffle(cat.raw.series.slice()), episodes: rr.shuffle(cat.raw.episodes.slice()), boxsets: rr.shuffle(cat.raw.boxsets.slice()) };
      const a = JSON.stringify(enc({ m: norm.movies, s: norm.series }));
      const n2 = index.normalizeCatalog(shuffled);
      // dedupe sortiert nach Id; die Roh-Duplikate sind exakt/ähnlich → nur prüfen, wenn Duplikate gleichgeordnet sind
      if (a !== JSON.stringify(enc({ m: n2.movies, s: n2.series }))) throw new Error('normalizeCatalog hängt von der Eingabereihenfolge ab');
    }

    for (const variant of cat.variants) {
      const winKeys = variant.windows || ['w0', 'w1', 'w2', 'w3'];
      for (const wk of winKeys) {
        const win = WINDOWS[wk];
        const res = await runWindow(cat.raw, boxContents, variant, win, { prepare: variant.prepare !== false });
        if (cname === 'normal' && variant.name === 'default' && wk === 'w0') {
          const res2 = await runWindow(cat.raw, boxContents, variant, win, { reverse: true });
          for (const no of Object.keys(res.plans)) for (const D of res.days) {
            if (JSON.stringify(res.plans[no][D]) !== JSON.stringify(res2.plans[no][D])) throw new Error(`Plan hängt von der Aufrufreihenfolge ab: Sender ${no} Tag ${D}`);
          }
        }
        // je Datei höchstens ~4,5 MB: ggf. nach Sendern aufteilen
        const chNos = Array.from({ length: 18 }, (_, i) => i + 1);
        const emit = (nos, part) => {
          const body = {
            catalog: cname, variant: variant.name, window: wk, prepared: variant.prepare !== false, config: variant.config === undefined ? { $: 'undefined' } : variant.config,
            days: res.days, prepareCalls: res.calls, part,
            channels: Object.fromEntries(res.days.map((D) => [D, res.chans[D]])),
            plans: Object.fromEntries(nos.map((no) => [no, Object.fromEntries(res.days.map((D) => [D, res.plans[no][D]]))])),
          };
          const size = Buffer.byteLength(JSON.stringify(enc(body)));
          if (size > 4.5e6 && nos.length > 1) {
            const h = nos.length >> 1;
            emit(nos.slice(0, h), part * 2);
            emit(nos.slice(h), part * 2 + 1);
          } else writeJson(`plans-${cname}-${variant.name}-${wk}-p${part}.json`.replace(/-p1\.json$/, '.json'), body);
        };
        emit(chNos, 1);
        if (variant.name === 'default' && wk === 'w0') nowFiles.push({ cname, res, cat, boxContents });
      }
    }
  }

  // nowOn / upcoming (Stichzeitpunkte). Eigener Scheduler, vorher prepare für die Tage.
  for (const { cname, res } of nowFiles) {
    const sched = res.sched;
    const D0 = WINDOWS.w0.from;
    const ts = [];
    for (const dd of [0, 3, 6, 13]) {
      const b = T.dayBounds(D0 + dd);
      ts.push(b.start, b.start + 1, b.start + 12 * 36e5 + 34 * 6e4 + 56789, b.a2015 - 1, b.a2015, b.a2200, b.a2300, b.end - 1);
    }
    const nowC = [];
    for (let no = 1; no <= 18; no++) {
      const list = [...ts];
      // Eintragsgrenzen des Tages D0+1
      const pl = sched.planDay(no, D0 + 1);
      for (let i = 0; i < pl.length; i += Math.max(1, Math.floor(pl.length / 5))) list.push(pl[i].start, pl[i].end - 1, pl[i].end);
      for (const t of list) {
        const n = sched.nowOn(no, t);
        nowC.push({ no, t, nowOn: n, up3: sched.upcoming(no, t, 3), ...([1, 3, 4, 5, 17, 18].includes(no) ? { up12: sched.upcoming(no, t, 12) } : {}) });
      }
    }
    // Zeitumstellung: Stichzeitpunkte rund um die Umschaltung (Katalog hat in w1..w3 keine Prepare-Daten außer wo geplant → eigene Pläne werden berechnet)
    const dst = [];
    for (const tt of [Date.UTC(2026, 9, 25, 0, 59, 59, 999), Date.UTC(2026, 9, 25, 1), Date.UTC(2027, 2, 28, 0, 59, 59, 999), Date.UTC(2027, 2, 28, 1), Date.UTC(2027, 9, 31, 0, 59, 59, 999), Date.UTC(2027, 9, 31, 1), Date.UTC(2027, 2, 27, 4, 0, 0), Date.UTC(2027, 2, 27, 3, 59, 59, 999)]) {
      for (const no of [1, 3, 4, 5, 17, 18]) {
        const sc = index.createScheduler(index.normalizeCatalog(cats[cname].raw));
        await sc.prepare([]);
        dst.push({ no, t: tt, nowOn: sc.nowOn(no, tt), up3: sc.upcoming(no, tt, 3) });
      }
    }
    const extra = {};
    if (cname === 'thin') { // lange Vorschau über die 3-Tage-Grenze
      const sc = index.createScheduler(index.normalizeCatalog(cats.thin.raw));
      extra.longUpcoming = [];
      for (let no = 1; no <= 18; no += 1) for (const t of [WINDOWS.w0.from * 864e5 + 8 * 36e5, WINDOWS.w0.from * 864e5 + 30 * 36e5]) extra.longUpcoming.push({ no, t, n: 80, out: sc.upcoming(no, t, 80) });
    }
    const errs = [];
    const sc2 = index.createScheduler(index.normalizeCatalog(cats[cname].raw));
    for (const [no, D] of [[0, WINDOWS.w0.from], [19, WINDOWS.w0.from], [1.5, WINDOWS.w0.from], [1, 20000.5], ['3', WINDOWS.w0.from], ['x', WINDOWS.w0.from], [1, '20000'], [1, NaN], [-1, 5]]) {
      let res2;
      try { const p = sc2.planDay(no, D); res2 = { ok: true, firstType: p[0].type }; } catch (e) { res2 = { ok: false, name: e.name, message: e.message }; }
      errs.push({ no, D, res: res2 });
    }
    writeJson(`now-${cname}.json`, { catalog: cname, cases: nowC, dst, errors: errs, ...extra });
  }

  meta.files = written.slice();
  fs.writeFileSync(path.join(OUT, 'meta.json'), JSON.stringify(meta, null, 1) + '\n');
}

await main();
console.log('geschrieben:', written.length + 1, 'Dateien nach', path.relative(process.cwd(), OUT) || '.');
for (const f of written) console.log(String(f.bytes).padStart(9), f.name);

#!/usr/bin/env node
// Extrahiert die Design-Tokens der Web-Vorlage (CSS) und erzeugt daraus
// crates/lfx-ui/ui/theme/tokens.slint. Keine Abhängigkeiten, deterministisch.
//
// Aufruf:  node tools/tokens/extract.mjs [Pfad/zur/Vorlage]
// Vorlage: Standard ../ (Elternordner dieses Repos = Web-Theme), oder Umgebungsvariable
//          LFX_THEME_SRC, oder erstes Argument. Erwartet <Vorlage>/src/css/...

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..');
const srcRoot = resolve(process.argv[2] || process.env.LFX_THEME_SRC || join(repoRoot, '..'));
const outFile = join(repoRoot, 'crates', 'lfx-ui', 'ui', 'theme', 'tokens.slint');

/** Referenzbreite (CSS-px) zum Auflösen von vw/clamp() in feste Längen. */
const REF_W = 1920;

// ---------------------------------------------------------------- CSS lesen

const stripComments = (s) => s.replace(/\/\*[\s\S]*?\*\//g, '');

/** Zerlegt CSS in Regeln { sel, decls: Map, media: string|null }. Nur eine Verschachtelungsebene (@media). */
function parseCss(text) {
  text = stripComments(text);
  const rules = [];
  let i = 0;
  const n = text.length;
  const readBlock = (start) => {
    // start zeigt auf '{'; liefert [inhalt, indexNach]
    let depth = 0;
    for (let j = start; j < n; j++) {
      if (text[j] === '{') depth++;
      else if (text[j] === '}') {
        depth--;
        if (depth === 0) return [text.slice(start + 1, j), j + 1];
      }
    }
    throw new Error('Unausgeglichene Klammern');
  };
  const parseDecls = (body) => {
    const m = new Map();
    for (const part of body.split(';')) {
      const k = part.indexOf(':');
      if (k < 0) continue;
      const name = part.slice(0, k).trim();
      if (!name.startsWith('--')) continue;
      m.set(name, part.slice(k + 1).trim().replace(/\s+/g, ' '));
    }
    return m;
  };
  const walk = (src, media) => {
    let p = 0;
    while (p < src.length) {
      const b = src.indexOf('{', p);
      if (b < 0) break;
      const head = src.slice(p, b).trim();
      // inneren Block finden
      let depth = 0, e = b;
      for (; e < src.length; e++) {
        if (src[e] === '{') depth++;
        else if (src[e] === '}') { depth--; if (depth === 0) break; }
      }
      const inner = src.slice(b + 1, e);
      if (head.startsWith('@media')) walk(inner, head);
      else if (!head.startsWith('@')) rules.push({ sel: head, decls: parseDecls(inner), media });
      p = e + 1;
    }
  };
  walk(text, null);
  void i; void readBlock;
  return rules;
}

const readCss = (rel) => parseCss(readFileSync(join(srcRoot, 'src', 'css', rel), 'utf8'));

// Selektor-Prüfung: Kern gilt für :root, Designs für ihre Klassen-Selektoren.
const selMatches = (sel, id) => {
  const parts = sel.split(',').map((s) => s.trim());
  if (id === 'flix') return parts.some((s) => s === ':root' || s === '[data-theme]');
  return parts.some((s) => s.includes(`lfx-theme-${id}`) && /^html\.lfx-js\.lfx-theme-[a-z]+:not\(\.layout-tv\)$/.test(s));
};

// ---------------------------------------------------------------- Media-Queries

/** Liest (min-width: Npx) / (max-width: Npx) → {min,max}. */
function mediaRange(media) {
  const r = { min: 0, max: Infinity };
  for (const m of media.matchAll(/\((min|max)-width:\s*([\d.]+)px\)/g)) {
    if (m[1] === 'min') r.min = Math.max(r.min, +m[2]);
    else r.max = Math.min(r.max, +m[2]);
  }
  // max-width 799 = „< 800“ → obere Grenze exklusiv als +1
  if (Number.isFinite(r.max)) r.max = r.max + 1;
  return r;
}

// ---------------------------------------------------------------- Wert-Auflösung

/** Löst var(--x[, fallback]) rekursiv gegen eine Map auf. Gibt null bei Unauflösbarem. */
function resolveVars(val, scope, depth = 0) {
  if (depth > 8) return null;
  let bad = false;
  const out = val.replace(/var\(\s*(--[\w-]+)\s*(?:,\s*([^)]*))?\)/g, (_, name, fb) => {
    if (scope.has(name)) {
      const r = resolveVars(scope.get(name), scope, depth + 1);
      if (r === null) { bad = true; return ''; }
      return r;
    }
    if (fb !== undefined) return fb.trim();
    bad = true;
    return '';
  });
  return bad ? null : out;
}

const fmt = (x) => {
  const s = (Math.round(x * 1e6) / 1e6).toString();
  return s === '-0' ? '0' : s;
};

/** Zahl + Einheit → px (rem = 16px, vw gegen REF_W). */
function toPx(tok) {
  tok = tok.trim();
  let m;
  if ((m = tok.match(/^(-?[\d.]+)px$/))) return +m[1];
  if ((m = tok.match(/^(-?[\d.]+)rem$/))) return +m[1] * 16;
  if ((m = tok.match(/^(-?[\d.]+)vw$/))) return (+m[1] * REF_W) / 100;
  if (/^-?0(\.0+)?$/.test(tok)) return 0;
  return null;
}

/** Länge inkl. clamp(min, pref, max) bei Referenzbreite → px oder null. */
function lengthPx(val) {
  const c = val.match(/^clamp\(\s*([^,]+),\s*([^,]+),\s*([^)]+)\)$/);
  if (c) {
    const [a, b, d] = [toPx(c[1]), toPx(c[2]), toPx(c[3])];
    if (a === null || b === null || d === null) return null;
    return Math.min(Math.max(b, a), d);
  }
  return toPx(val);
}

// ---------------------------------------------------------------- Farben

const hex2 = (n) => Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, '0');

/** CSS-Farbe → Slint-Literal oder null. */
function colorLit(val) {
  val = val.trim().toLowerCase();
  if (val === 'transparent') return 'transparent';
  let m;
  if ((m = val.match(/^#([0-9a-f]{3})$/))) return '#' + [...m[1]].map((c) => c + c).join('');
  if ((m = val.match(/^#([0-9a-f]{4})$/))) return '#' + [...m[1]].map((c) => c + c).join('');
  if ((m = val.match(/^#([0-9a-f]{6}|[0-9a-f]{8})$/))) return '#' + m[1];
  if ((m = val.match(/^rgba?\(\s*([^)]+)\)$/))) {
    const nums = m[1].split(/[\s,/]+/).filter(Boolean).map(Number);
    if (nums.length < 3 || nums.some(Number.isNaN)) return null;
    const [r, g, b, a] = nums;
    if (a === undefined || a === 1) return '#' + hex2(r) + hex2(g) + hex2(b);
    return `rgba(${fmt(r)}, ${fmt(g)}, ${fmt(b)}, ${fmt(a)})`;
  }
  return null;
}

/** Splittet an Kommas auf oberster Klammerebene. */
function splitTop(s, sep = ',') {
  notes.push('LENZ//84: Verlauf --lfx-retro-scan (Scanlines) hat kein Slint-Gegenstück als Token; Umsetzung später als Bild/Shader');
const out = [];
  let d = 0, cur = '';
  for (const ch of s) {
    if (ch === '(') d++;
    if (ch === ')') d--;
    if (ch === sep && d === 0) { out.push(cur.trim()); cur = ''; } else cur += ch;
  }
  if (cur.trim()) out.push(cur.trim());
  return out;
}

/** Schattenliste → Schichten {x,y,blur,spread,color,inset}. */
function parseShadows(val) {
  const layers = [];
  for (const layer of splitTop(val)) {
    // Farbe kann vorn oder hinten stehen
    const colorRe = /(rgba?\([^)]*\)|#[0-9a-fA-F]{3,8})/;
    const cm = layer.match(colorRe);
    if (!cm) return null;
    const rest = layer.replace(cm[0], ' ').trim();
    const inset = /\binset\b/.test(rest);
    const nums = rest.replace(/\binset\b/, '').trim().split(/\s+/).map((t) => toPx(t));
    if (nums.length < 2 || nums.length > 4 || nums.some((x) => x === null)) return null;
    const color = colorLit(cm[0]);
    if (!color) return null;
    layers.push({ x: nums[0], y: nums[1], blur: nums[2] ?? 0, spread: nums[3] ?? 0, color, inset });
  }
  return layers;
}

// ---------------------------------------------------------------- Token-Spezifikation

// [Feldname, [CSS-Variablen in Rangfolge], Typ]
// Typen: color length duration int float string easing shadow
const FONT_MAP = {
  'LenzFlix Sans': 'Inter',
  'LenzPlus Sans': 'Figtree',
  'LenzPrime Sans': 'Source Sans 3',
  'LenzRetro Mono': 'Space Mono',
  'LenzRetro Pixel': 'VT323',
};

const SPEC = [
  ['// Jellyfin-Palette (MUI)'],
  ['jf-primary-main', ['--jf-palette-primary-main'], 'color'],
  ['jf-primary-light', ['--jf-palette-primary-light'], 'color'],
  ['jf-primary-dark', ['--jf-palette-primary-dark'], 'color'],
  ['jf-primary-contrast', ['--jf-palette-primary-contrastText'], 'color'],
  ['jf-secondary-main', ['--jf-palette-secondary-main'], 'color'],
  ['jf-bg-default', ['--jf-palette-background-default'], 'color'],
  ['jf-bg-paper', ['--jf-palette-background-paper'], 'color'],
  ['jf-appbar-bg', ['--jf-palette-AppBar-defaultBg'], 'color'],
  ['jf-text-primary', ['--jf-palette-text-primary'], 'color'],
  ['jf-text-secondary', ['--jf-palette-text-secondary'], 'color'],
  ['jf-divider', ['--jf-palette-divider'], 'color'],
  ['jf-snackbar-bg', ['--jf-palette-SnackbarContent-bg'], 'color'],
  ['jf-filled-input-bg', ['--jf-palette-FilledInput-bg'], 'color'],
  ['jf-button-bg', ['--jf-palette-Button-inheritContainedBg'], 'color'],
  ['jf-button-hover-bg', ['--jf-palette-Button-inheritContainedHoverBg'], 'color'],
  ['jf-card-radius', ['--jf-card-borderRadius'], 'length'],
  ['jf-shape-radius', ['--jf-shape-borderRadius'], 'length'],
  ['// Farben (--lfx-*)'],
  ['red', ['--lfx-red'], 'color'],
  ['red-hover', ['--lfx-red-hover'], 'color'],
  ['red-active', ['--lfx-red-active'], 'color'],
  ['accent', ['--lfx-accent', '--lfx-red'], 'color'],
  ['link', ['--lfx-link', '--lfx-red-hover'], 'color'],
  ['progress', ['--lfx-progress', '--lfx-accent', '--lfx-red'], 'color'],
  ['bg', ['--lfx-bg'], 'color'],
  ['bg-deep', ['--lfx-bg-deep'], 'color'],
  ['surface', ['--lfx-surface'], 'color'],
  ['surface-2', ['--lfx-surface-2'], 'color'],
  ['surface-3', ['--lfx-surface-3'], 'color'],
  ['border', ['--lfx-border'], 'color'],
  ['text', ['--lfx-text'], 'color'],
  ['text-body', ['--lfx-text-body'], 'color'],
  ['text-muted', ['--lfx-text-muted'], 'color'],
  ['text-soft', ['--lfx-text-soft'], 'color'],
  ['gold', ['--lfx-gold'], 'color'],
  ['gold-hover', ['--lfx-gold-hover'], 'color'],
  ['green', ['--lfx-green'], 'color'],
  ['green-soft', ['--lfx-green-soft'], 'color'],
  ['glass', ['--lfx-glass'], 'color'],
  ['glass-hover', ['--lfx-glass-hover'], 'color'],
  ['pill-active', ['--lfx-pill-active'], 'color'],
  ['pill-soft', ['--lfx-pill-soft'], 'color'],
  ['icon-circle', ['--lfx-icon-circle'], 'color'],
  ['icon-circle-border', ['--lfx-icon-circle-border'], 'color'],
  ['progress-rail', ['--lfx-progress-rail'], 'color'],
  ['ambient', ['--lfx-ambient'], 'color'],
  ['// Typografie'],
  ['font-family', ['--lfx-font'], 'font'],
  ['fs-nav', ['--lfx-fs-nav'], 'length'],
  ['fs-row-title', ['--lfx-fs-row-title'], 'length'],
  ['fs-meta', ['--lfx-fs-meta'], 'length'],
  ['fs-button', ['--lfx-fs-button'], 'length'],
  ['fs-badge', ['--lfx-fs-badge'], 'length'],
  ['fs-body', ['--lfx-fs-body'], 'length'],
  ['fw-nav', ['--lfx-fw-nav'], 'int'],
  ['fw-row-title', ['--lfx-fw-row-title'], 'int'],
  ['fw-button', ['--lfx-fw-button'], 'int'],
  ['// Maße'],
  ['gutter', ['--lfx-gutter'], 'length'],
  ['header-h', ['--lfx-header-h'], 'length'],
  ['nav-pill-h', ['--lfx-nav-pill-h'], 'length'],
  ['card-gap', ['--lfx-card-gap'], 'length'],
  ['cards-visible', ['--lfx-cards-visible'], 'int'],
  ['card-peek', ['--lfx-card-peek'], 'length'],
  ['card-radius', ['--lfx-card-radius'], 'length'],
  ['row-title-gap', ['--lfx-row-title-gap'], 'length'],
  ['row-gap', ['--lfx-row-gap'], 'length'],
  ['billboard-radius', ['--lfx-billboard-radius'], 'length'],
  ['billboard-ratio', ['--lfx-billboard-ratio'], 'ratio'],
  ['btn-h', ['--lfx-btn-h'], 'length'],
  ['btn-radius', ['--lfx-btn-radius'], 'length'],
  ['btn-gap', ['--lfx-btn-gap'], 'length'],
  ['badge-h', ['--lfx-badge-h'], 'length'],
  ['popup-radius', ['--lfx-popup-radius'], 'length'],
  ['modal-radius', ['--lfx-modal-radius'], 'length'],
  ['// Bewegung'],
  ['ease', ['--lfx-ease'], 'easing'],
  ['ease-out', ['--lfx-ease-out'], 'easing'],
  ['dur-fast', ['--lfx-dur-fast'], 'duration'],
  ['dur', ['--lfx-dur'], 'duration'],
  ['hover-delay', ['--lfx-hover-delay'], 'duration'],
  ['// Schatten (jeweils die erste Schicht mit Unschärfe; Rest siehe Kommentar je Design)'],
  ['shadow-card', ['--lfx-shadow-card'], 'shadow'],
  ['shadow-modal', ['--lfx-shadow-modal'], 'shadow'],
  ['// LENZ+ (plus-*, in anderen Designs Nullwerte)'],
  ['plus-nav-idle', ['--lfx-plus-nav-idle'], 'color'],
  ['plus-circle', ['--lfx-plus-circle'], 'color'],
  ['plus-menu-bg', ['--lfx-plus-menu-bg'], 'color'],
  ['plus-menu-border', ['--lfx-plus-menu-border'], 'color'],
  ['plus-letter-spacing', ['--lfx-plus-ls'], 'length'],
  ['plus-letter-spacing-bold', ['--lfx-plus-ls-bold'], 'length'],
  ['// LENZPRIME (prime-*, in anderen Designs Nullwerte)'],
  ['prime-glass', ['--lfx-prime-glass'], 'color'],
  ['prime-chip', ['--lfx-prime-chip'], 'color'],
  ['prime-hover-bg', ['--lfx-prime-hover-bg'], 'color'],
  ['prime-ink', ['--lfx-prime-ink'], 'color'],
  ['prime-gutter', ['--lfx-prime-gutter'], 'length'],
  ['prime-header-h', ['--lfx-prime-header-h'], 'length'],
  ['prime-shadow', ['--lfx-prime-shadow'], 'shadow'],
  ['// LENZ//84 (retro-*, in anderen Designs Nullwerte)'],
  ['retro-amber', ['--lfx-retro-amber'], 'color'],
  ['retro-amber-hi', ['--lfx-retro-amber-hi'], 'color'],
  ['retro-amber-lo', ['--lfx-retro-amber-lo'], 'color'],
  ['retro-amber-dim', ['--lfx-retro-amber-dim'], 'color'],
  ['retro-hot', ['--lfx-retro-hot'], 'color'],
  ['retro-glitch-a', ['--lfx-retro-glitch-a'], 'color'],
  ['retro-glitch-b', ['--lfx-retro-glitch-b'], 'color'],
  ['retro-bg', ['--lfx-retro-bg'], 'color'],
  ['retro-bg-2', ['--lfx-retro-bg-2'], 'color'],
  ['retro-panel', ['--lfx-retro-panel'], 'color'],
  ['retro-line', ['--lfx-retro-line'], 'color'],
  ['retro-ink', ['--lfx-retro-ink'], 'color'],
  ['retro-glow', ['--lfx-retro-glow'], 'shadow'],
  ['retro-glow-strong', ['--lfx-retro-glow-strong'], 'shadow'],
  ['retro-box-glow', ['--lfx-retro-box-glow'], 'shadow'],
  ['retro-font-pixel', ['--lfx-retro-font-pixel'], 'font'],
  ['retro-font-mono', ['--lfx-retro-font-mono'], 'font'],
  ['retro-ease-crt', ['--lfx-retro-ease-crt'], 'easing'],
  ['retro-dur-glitch', ['--lfx-retro-dur-glitch'], 'duration'],
  ['retro-gutter', ['--lfx-retro-gutter'], 'length'],
];

const TYPE_NAME = {
  color: 'color', length: 'length', duration: 'duration', int: 'int', float: 'float',
  ratio: 'float', string: 'string', font: 'string', easing: 'easing', shadow: 'Shadow',
};
const DEFAULT_LIT = {
  color: 'transparent', length: '0px', duration: '0ms', int: '0', float: '0', ratio: '0', string: '""',
  font: '""', easing: 'linear', shadow: '{ x: 0px, y: 0px, blur: 0px, spread: 0px, color: transparent }',
};

// ---------------------------------------------------------------- Design bauen

const DESIGNS = [
  { id: 'flix', name: 'Flix', file: 'tokens.css', label: 'LENZFLIX' },
  { id: 'plus', name: 'Plus', file: 'themes/plus/tokens.css', label: 'LENZ+' },
  { id: 'prime', name: 'Prime', file: 'themes/prime/tokens.css', label: 'LENZPRIME' },
  { id: 'retro', name: 'Retro', file: 'themes/retro/tokens.css', label: 'LENZ//84' },
];

const notes = []; // Dokumentation nicht (voll) übersetzbarer Werte

function buildDesign(d, baseScope) {
  const rules = readCss(d.file).filter((r) => selMatches(r.sel, d.id));
  const scope = new Map(baseScope || []); // Designs erben die Kern-Tokens (Kaskade)
  const mediaRules = [];
  const own = new Map(); // eigene Basiswerte des Designs (schlagen Kern-Media-Queries per Spezifität)
  for (const r of rules) {
    if (r.media) mediaRules.push(r);
    else for (const [k, v] of r.decls) { scope.set(k, v); own.set(k, v); }
  }
  return { scope, mediaRules, own };
}

const base = buildDesign(DESIGNS[0]);
const built = new Map([['flix', base]]);
for (const d of DESIGNS.slice(1)) built.set(d.id, buildDesign(d, base.scope));

/** Die zu `id` gehörenden @media-Regeln inkl. geerbter (Design ohne eigene → Kern). */
function effectiveMedia(id, varName) {
  const own = built.get(id).mediaRules.filter((r) => r.decls.has(varName));
  if (own.length) return { rules: own, from: id };
  return { rules: base.mediaRules.filter((r) => r.decls.has(varName)), from: 'flix' };
}

/** Wert (Länge/int) bei Breite w nach CSS-Kaskade; spätere Regel gewinnt. */
function valueAt(id, varName, w, baseVal) {
  // Kaskade: Kern-Basis < Kern-Media < Design-Basis (höhere Spezifität) < Design-Media.
  const core = base;
  const des = built.get(id);
  const apply = (src, v) => {
    for (const r of src.mediaRules) {
      if (!r.decls.has(varName)) continue;
      const { min, max } = mediaRange(r.media);
      if (w >= min && w < max) v = r.decls.get(varName);
    }
    return v;
  };
  let v = core.scope.get(varName) ?? baseVal;
  v = apply(core, v);
  if (id !== 'flix') {
    if (des.own.has(varName)) v = des.own.get(varName);
    v = apply(des, v);
  }
  return v;
}

function breakpointsFor(id, varName) {
  const set = new Set();
  const chain = id === 'flix' ? [base] : [base, built.get(id)];
  for (const src of chain) {
    for (const r of src.mediaRules) {
      if (!r.decls.has(varName)) continue;
      const { min, max } = mediaRange(r.media);
      if (min > 0) set.add(min);
      if (Number.isFinite(max)) set.add(max);
    }
  }
  return [...set].sort((a, b) => a - b);
}

function convert(d, field, vars, type, scope) {
  // erste definierte Variable der Rangfolge
  let name = vars.find((v) => scope.has(v));
  if (!name) return { lit: DEFAULT_LIT[type], missing: true };
  const raw = scope.get(name);
  const val = resolveVars(raw, scope);
  const fail = (why) => {
    notes.push(`${d.label}: ${name} (${field}) = "${raw}" nicht übersetzbar (${why}); Nullwert gesetzt`);
    return { lit: DEFAULT_LIT[type] };
  };
  if (val === null) return fail('var() nicht auflösbar');
  const viaFallback = name !== vars[0];
  const via = viaFallback ? name : null;
  if (/calc\(/.test(val)) return fail('calc()');
  switch (type) {
    case 'color': {
      if (field === 'ambient') {
        const m = val.match(/^(\d+)\s+(\d+)\s+(\d+)$/);
        if (m) return { lit: `rgb(${m[1]}, ${m[2]}, ${m[3]})`, via };
      }
      const c = colorLit(val);
      return c ? { lit: c, via } : fail('Farbformat');
    }
    case 'length': {
      const px = lengthPx(val);
      if (px === null) return fail('Längenformat');
      const lit = `${fmt(px)}px`;
      if (/vw|clamp/.test(val)) notes.push(`${d.label}: ${name} (${field}) = "${val}" bei Referenzbreite ${REF_W} px aufgelöst: ${lit}`);
      return { lit, via };
    }
    case 'duration': {
      const m = val.match(/^([\d.]+)(ms|s)$/);
      if (!m) return fail('Dauerformat');
      return { lit: `${fmt(m[2] === 's' ? +m[1] * 1000 : +m[1])}ms`, via };
    }
    case 'int': {
      const m = val.match(/^-?\d+$/);
      return m ? { lit: val, via } : fail('Ganzzahl');
    }
    case 'ratio': {
      const m = val.match(/^([\d.]+)\s*\/\s*([\d.]+)$/);
      return m ? { lit: fmt(+m[1] / +m[2]), via } : fail('Seitenverhältnis');
    }
    case 'easing': {
      const m = val.match(/^cubic-bezier\(\s*([\d.\-]+)\s*,\s*([\d.\-]+)\s*,\s*([\d.\-]+)\s*,\s*([\d.\-]+)\s*\)$/);
      return m ? { lit: `cubic-bezier(${fmt(+m[1])}, ${fmt(+m[2])}, ${fmt(+m[3])}, ${fmt(+m[4])})`, via } : fail('Easing');
    }
    case 'font': {
      const first = splitTop(val)[0].replace(/^["']|["']$/g, '');
      const fam = FONT_MAP[first] || first;
      return { lit: `"${fam}"`, via, comment: `CSS: ${val}` };
    }
    case 'shadow': {
      const layers = parseShadows(val);
      if (!layers || !layers.length) return fail('Schattenformat');
      const pick = layers.find((l) => l.blur > 0 && !l.inset) || layers.find((l) => !l.inset) || layers[0];
      const lit = `{ x: ${fmt(pick.x)}px, y: ${fmt(pick.y)}px, blur: ${fmt(pick.blur)}px, spread: ${fmt(pick.spread)}px, color: ${pick.color} }`;
      let comment = null;
      if (layers.length > 1) {
        comment = `${layers.length} Schichten, übernommen wird die ${layers.indexOf(pick) + 1}.; Original: ${val}`;
        notes.push(`${d.label}: ${name} (${field}) hat ${layers.length} Schatten-Schichten; Slint kennt nur eine (Schicht ${layers.indexOf(pick) + 1} übernommen), Spread wird von Slint ignoriert`);
      }
      return { lit, via, comment };
    }
  }
  return fail('unbekannter Typ');
}

// ---------------------------------------------------------------- Ausgabe

const out = [];
const w = (s = '') => out.push(s);

w('// Generiert von tools/tokens/extract.mjs – nicht von Hand ändern.');
w('// Quelle: src/css/tokens.css und src/css/themes/{plus,prime,retro}/tokens.css der Web-Vorlage (store bewusst ausgelassen).');
w('//');
w('// Aufbau: ein `DesignTokens`-Struct (alle Felder für alle Designs) und je Design ein Global');
w('// (TokensFlix, TokensPlus, TokensPrime, TokensRetro) mit `out property <DesignTokens> v`.');
w('// Ein späteres Theme-Global wählt per Design-Enum (flix, plus, prime, retro) das passende Global.');
w('// Designs erben wie in der Web-Kaskade die Kern-Tokens (LENZFLIX); Felder, die ein Design nicht');
w('// kennt (z. B. retro-* in LENZ+), tragen Nullwerte (transparent / 0px / "").');
w('// Zusätzlich je Global: `cards-visible-at(width)` und `gutter-at(width)` (Breakpoints der Media-Queries).');
w('//');
w(`// Konventionen: rem = 16px; vw und clamp() werden bei ${REF_W} px Referenzbreite zu festen Längen aufgelöst`);
w('// (die Breitenabhängigkeit liefert gutter-at()). Schriftfamilien sind die echten Familiennamen der');
w('// eingebetteten Schriften (z. B. "Inter" statt CSS-Name "LenzFlix Sans"). Schatten: Slint kennt nur eine');
w('// Schicht ohne Spread, übernommen wird die erste Schicht mit Unschärfe.');
w('// Nicht übernommen: --*Channel-Tokens (Redundanz), --lfx-css (Web-Marker), --lfx-retro-scan (Verlauf).');
w('');
w('export struct Shadow {');
w('    x: length,');
w('    y: length,');
w('    blur: length,');
w('    spread: length,');
w('    color: color,');
w('}');
w('');
w('export struct DesignTokens {');
for (const s of SPEC) {
  if (s.length === 1) { w(`    ${s[0]}`); continue; }
  w(`    ${s[0]}: ${TYPE_NAME[s[2]]},`);
}
w('}');

const widthsToTest = (bps) => {
  // Repräsentative Breite je Intervall
  const edges = [0, ...bps];
  return edges.map((e, i) => ({ lo: e, hi: i + 1 < edges.length ? edges[i + 1] : Infinity, rep: e }));
};

function widthFn(d, field, varName, ret, baseScope) {
  const scope = built.get(d.id).scope;
  const baseVal = scope.get(varName);
  const bps = breakpointsFor(d.id, varName);
  const lines = [];
  const toLit = (raw) => {
    const val = resolveVars(raw, scope);
    if (val === null) return DEFAULT_LIT[ret];
    if (ret === 'int') return val;
    // Länge: vw → Faktor auf Breite, px → Literal
    const m = val.match(/^(-?[\d.]+)vw$/);
    if (m) return `w * ${fmt(+m[1] / 100)}`;
    const px = lengthPx(val);
    return px === null ? '0px' : `${fmt(px)}px`;
  };
  const ivs = widthsToTest(bps);
  const vals = ivs.map((iv) => toLit(valueAt(d.id, varName, iv.rep, baseVal)));
  // aufeinanderfolgende gleiche Werte zusammenfassen
  const merged = [];
  ivs.forEach((iv, i) => {
    const last = merged[merged.length - 1];
    if (last && last.val === vals[i]) last.hi = iv.hi;
    else merged.push({ lo: iv.lo, hi: iv.hi, val: vals[i] });
  });
  const retT = ret === 'int' ? 'int' : 'length';
  lines.push(`    // ${field}: Breakpoints aus den Media-Queries (CSS-px)`);
  lines.push(`    pure function ${field}-at(w: length) -> ${retT} {`);
  merged.forEach((m, i) => {
    if (i === merged.length - 1) lines.push(`        return ${m.val};`);
    else lines.push(`        if (w < ${fmt(m.hi)}px) { return ${m.val}; }`);
  });
  lines.push('    }');
  return lines;
}

for (const d of DESIGNS) {
  const { scope } = built.get(d.id);
  w('');
  w(`// ---- ${d.label} (${d.id}) ----`);
  w(`export global Tokens${d.name} {`);
  w('    out property <DesignTokens> v: {');
  for (const s of SPEC) {
    if (s.length === 1) { w(`        ${s[0]}`); continue; }
    const [field, vars, type] = s;
    const r = convert(d, field, vars, type, scope);
    const tail = [r.via ? `Rückfall auf ${r.via}` : null, r.comment || null].filter(Boolean).join('; ');
    w(`        ${field}: ${r.lit},${tail ? ` // ${tail}` : ''}`);
  }
  w('    };');
  for (const l of widthFn(d, 'cards-visible', '--lfx-cards-visible', 'int')) w(l);
  for (const l of widthFn(d, 'gutter', '--lfx-gutter', 'length')) w(l);
  w('}');
}

if (notes.length) {
  w('');
  w('// ---- Hinweise zur Übersetzung (nicht oder nur genähert übersetzbar) ----');
  for (const n of [...new Set(notes)]) w(`// - ${n}`);
}

mkdirSync(dirname(outFile), { recursive: true });
writeFileSync(outFile, out.join('\n') + '\n', 'utf8');
console.log(`tokens.slint geschrieben: ${out.length} Zeilen, ${notes.length} Hinweise`);

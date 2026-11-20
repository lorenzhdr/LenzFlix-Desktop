// Extrahiert die eigenen Inline-Icons der Web-Vorlage (src/js/core/icons.js) als eigenständige SVG-Dateien.
// Aufruf: node tools/icons/extract.mjs [Web-Verzeichnis]   (sonst LFX_THEME_SRC, sonst Elternordner `..`)
// Ausgabe: assets/icons/<name>.svg (kebab-case, ohne Präfix ICON_), weiß gefüllt (in Slint per `colorize` einfärben).
// Deterministisch: gleiche Eingabe ergibt byte-gleiche Ausgabe. Handgezeichnete Icons liegen direkt in assets/icons
// und werden hier nicht angefasst (nur Dateien, die aus icons.js stammen, werden geschrieben).
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..', '..');
const web = resolve(process.argv[2] ?? process.env.LFX_THEME_SRC ?? join(root, '..'));
const src = readFileSync(join(web, 'src', 'js', 'core', 'icons.js'), 'utf8');

// export const ICON_NAME = svg('...');  (auch über mehrere Zeilen, einfache Anführungszeichen)
const re = /export const ICON_([A-Z0-9_]+) = svg\(\s*'((?:[^'\\]|\\.)*)'\s*,?\s*\)/g;
const out = join(root, 'assets', 'icons');
mkdirSync(out, { recursive: true });

let n = 0;
for (const m of src.matchAll(re)) {
  const name = m[1].toLowerCase().replace(/_/g, '-');
  const body = m[2].replace(/\\'/g, "'");
  const svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#ffffff">' + body + '</svg>\n';
  writeFileSync(join(out, `${name}.svg`), svg);
  n += 1;
}
console.log(`${n} Icons nach assets/icons geschrieben`);

// Erzeugt tests/data/ambient_vectors.json aus der Web-Vorlage src/js/core/color.js.
// Aufruf: LENZFLIX_WEB_DIR=<Pfad zum Web-Repo> node tools/gen_ambient_vectors.mjs
// Die Pixel sind synthetisch (deterministischer Zufall), keine echten Cover.
import { pathToFileURL } from 'node:url';
import { writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const web = process.env.LENZFLIX_WEB_DIR;
if (!web) throw new Error('LENZFLIX_WEB_DIR fehlt');
const color = await import(pathToFileURL(join(web, 'src/js/core/color.js')).href);

// xorshift32, identisch zu reproduzieren, falls nötig.
let seed = 0x1234abcd;
const rnd = () => {
  seed ^= seed << 13; seed >>>= 0;
  seed ^= seed >>> 17;
  seed ^= seed << 5; seed >>>= 0;
  return seed;
};
const hex = (px) => Buffer.from(px).toString('hex');

function image(w, h, fn) {
  const px = new Uint8ClampedArray(w * h * 4);
  for (let i = 0; i < w * h; i++) {
    const [r, g, b, a] = fn(i % w, Math.floor(i / w));
    px.set([r, g, b, a], i * 4);
  }
  return px;
}

const cases = [];
const add = (name, px) => {
  const dom = color.dominantColor(px);
  cases.push({
    name,
    rgba: hex(px),
    dominant: dom,
    ambient: color.ambientFromPixels(px),
    clamped: dom ? color.clampAmbient(dom) : null,
  });
};

add('rauschen-16x16', image(16, 16, () => [rnd() & 255, rnd() & 255, rnd() & 255, 255]));
add('rauschen-33x7', image(33, 7, () => [rnd() & 255, rnd() & 255, rnd() & 255, 255]));
add('rauschen-halbtransparent', image(20, 20, () => [rnd() & 255, rnd() & 255, rnd() & 255, rnd() & 255]));
add('alles-schwarz', image(8, 8, () => [0, 0, 0, 255]));
add('alles-weiss', image(8, 8, () => [255, 255, 255, 255]));
add('alles-transparent', image(8, 8, () => [200, 30, 30, 0]));
add('grau', image(8, 8, () => [128, 128, 128, 255]));
add('leer', new Uint8ClampedArray(0));
add('rot-blau-gleichstand', image(8, 8, (x) => (x < 4 ? [200, 20, 20, 255] : [20, 20, 200, 255])));
add('blau-mehrheit', image(10, 10, (x) => (x < 3 ? [220, 40, 40, 255] : [30, 60, 190, 255])));
add('verlauf', image(32, 32, (x, y) => [x * 8, y * 8, 255 - x * 8, 255]));
add('pastell', image(12, 12, (x, y) => [200 + (x % 3), 180 + (y % 4), 190, 255]));
add('dunkel-mit-akzent', image(16, 16, (x, y) => (x === 3 && y === 3 ? [250, 120, 10, 255] : [10, 10, 12, 255])));
add('teiltransparent-kante', image(10, 10, (x) => (x < 5 ? [10, 200, 90, 127] : [10, 90, 200, 128])));
add('grenze-helligkeit', image(8, 8, (x) => (x < 4 ? [20, 20, 20, 255] : [235, 235, 235, 255])));

const hsl = [];
for (let i = 0; i < 60; i++) {
  const r = rnd() & 255, g = rnd() & 255, b = rnd() & 255;
  hsl.push({ rgb: [r, g, b], hsl: color.rgbToHsl(r, g, b), clamped: color.clampAmbient([r, g, b]) });
}
for (const rgb of [[0, 0, 0], [255, 255, 255], [255, 0, 0], [0, 255, 0], [0, 0, 255], [128, 128, 128], [255, 255, 0], [1, 2, 3]]) {
  hsl.push({ rgb, hsl: color.rgbToHsl(...rgb), clamped: color.clampAmbient(rgb) });
}

const out = join(dirname(fileURLToPath(import.meta.url)), '../tests/data/ambient_vectors.json');
writeFileSync(out, JSON.stringify({ default: color.DEFAULT_AMBIENT, cases, hsl }, null, 1) + '\n');
console.log('geschrieben:', cases.length, 'Bilder,', hsl.length, 'HSL-Fälle');

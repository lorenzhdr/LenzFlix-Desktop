// Selbsttest des Generators: Determinismus (zweimal erzeugen -> byte-gleich), Zeitzonen-Unabhängigkeit und Gleichheit mit dem
// eingecheckten Stand. Aufruf: node tools/golden/selftest.mjs   (LENZFLIX_WEB_DIR wie bei gen.mjs)
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const COMMITTED = path.resolve(HERE, '../../crates/lfx-tv-schedule/tests/golden');
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'lfx-golden-'));
const gen = (name, env = {}) => {
  const out = path.join(tmp, name);
  execFileSync(process.execPath, [path.join(HERE, 'gen.mjs'), '--out', out], { env: { ...process.env, ...env }, stdio: 'pipe', maxBuffer: 1 << 28 });
  return out;
};
const list = (d) => fs.readdirSync(d).filter((f) => f.endsWith('.json')).sort();
// norm: für den Zeitzonenvergleich werden die bewusst lokalzeitabhängigen V8-Sonderfälle (parseDateLegacy) und die Dateigrößenliste ignoriert
function same(a, b, f, norm) {
  const x = fs.readFileSync(path.join(a, f));
  const y = fs.readFileSync(path.join(b, f));
  if (!norm) return x.equals(y);
  const strip = (buf) => { const j = JSON.parse(buf.toString()); delete j.parseDateLegacy; delete j.files; return JSON.stringify(j); };
  return strip(x) === strip(y);
}
function compare(a, b, label, norm = false) {
  const fa = list(a);
  const fb = list(b);
  let bad = 0;
  if (fa.join() !== fb.join()) { console.error(`${label}: Dateiliste verschieden`); bad++; }
  for (const f of fa) {
    if (!fb.includes(f)) continue;
    if (!same(a, b, f, norm)) { console.error(`${label}: ${f} verschieden`); bad++; }
  }
  console.log(`${label}: ${bad ? 'FEHLER (' + bad + ')' : 'ok (' + fa.length + ' Dateien byte-gleich)'}`);
  return bad;
}
let bad = 0;
const a = gen('a');
const b = gen('b');
bad += compare(a, b, 'Determinismus (Lauf 1 vs. 2)');
const c = gen('c', { GOLDEN_TZ_TEST: 'America/New_York' });
bad += compare(a, c, 'Zeitzone (UTC vs. America/New_York)', true);
const d = gen('d', { GOLDEN_TZ_TEST: 'Asia/Kolkata' });
bad += compare(a, d, 'Zeitzone (UTC vs. Asia/Kolkata)', true);
if (fs.existsSync(COMMITTED)) bad += compare(a, COMMITTED, 'Stand im Repo');
fs.rmSync(tmp, { recursive: true, force: true });
process.exit(bad ? 1 : 0);

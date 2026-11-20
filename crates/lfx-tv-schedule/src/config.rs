//! Admin-Belegung der Senderplätze 5 bis 16 (Port von `features/tvdata/config.js`).
//!
//! Das Auslesen der Quellen (Einstellungen, CSS-Variable) ist Sache der App; hier liegt nur der
//! reine Format-Parser. (Der Planer selbst hat zusätzlich einen eigenen, strengeren Parser,
//! siehe [`crate::plan`].)

use std::collections::BTreeMap;

use crate::jsval::{JsVal, is_js_ws, js_number, js_string_or_empty, js_trim};

const MIN_NO: i64 = 5;
const MAX_NO: i64 = 16;

fn strip_quotes(s: &str) -> String {
    let t = js_trim(s);
    t.trim_matches(|c: char| c == '"' || c == '\'' || is_js_ws(c))
        .to_string()
}

fn put(out: &mut BTreeMap<i64, String>, no: &JsVal, label: &str) {
    let n = js_number(no);
    if n.fract() != 0.0 || !n.is_finite() || n < MIN_NO as f64 || n > MAX_NO as f64 {
        return;
    }
    let l = strip_quotes(label);
    if !l.is_empty() {
        out.insert(n as i64, l);
    }
}

/// Objekt oder Zeichenkette zu `{ Platz: Genre-Bezeichnung }` (nur Plätze 5 bis 16, ungültige
/// Einträge entfallen).
pub fn parse_channel_config(raw: &JsVal) -> BTreeMap<i64, String> {
    let mut out = BTreeMap::new();
    match raw {
        JsVal::Obj(o) => {
            for (k, v) in crate::jsval::object_entries_ordered(o) {
                if let JsVal::Str(s) = v {
                    put(&mut out, &JsVal::Str(k.to_string()), s);
                }
            }
            return out;
        }
        JsVal::Arr(a) => {
            // Object.entries eines Arrays: Schlüssel "0", "1", ... (liegen außerhalb 5..16 nur
            // bei kurzen Listen nicht; deshalb regulär behandeln)
            for (i, v) in a.iter().enumerate() {
                if let JsVal::Str(s) = v {
                    put(&mut out, &JsVal::Num(i as f64), s);
                }
            }
            return out;
        }
        _ => {}
    }
    let s = strip_quotes(&js_string_or_empty(raw));
    if s.is_empty() {
        return out;
    }
    for part in s.split(';') {
        if let Some(i) = part.find('=') {
            put(
                &mut out,
                &JsVal::Str(js_trim(&part[..i]).to_string()),
                &part[i + 1..],
            );
        }
    }
    out
}

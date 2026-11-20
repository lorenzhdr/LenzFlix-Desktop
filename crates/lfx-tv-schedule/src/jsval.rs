//! Kleine Nachbildung von JavaScript-Werten und ihren Umwandlungsregeln (`Number(x)`, `String(x)`,
//! `JSON.stringify`, `trim`). Rohdaten des Servers werden damit so gelesen wie im Web-Planer.

use serde_json::Value;

/// Ein JavaScript-Wert (nur die Arten, die in JSON-Rohdaten vorkommen können, plus `undefined`).
#[derive(Debug, Clone, PartialEq)]
pub enum JsVal {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<JsVal>),
    /// Eigenschaften in Einfügereihenfolge.
    Obj(Vec<(String, JsVal)>),
}

static UNDEFINED: JsVal = JsVal::Undefined;

impl JsVal {
    /// Aus `serde_json::Value`. Die Marker-Objekte der Golden-Vektoren (`{"$": "undefined"}`,
    /// `NaN`, `Infinity`, `-Infinity`, `-0`, `u16`) werden zu den echten Werten aufgelöst
    /// (`u16` mit einzelnen Surrogaten wird verlustbehaftet nach UTF-8 gewandelt).
    pub fn from_json(v: &Value) -> JsVal {
        match v {
            Value::Null => JsVal::Null,
            Value::Bool(b) => JsVal::Bool(*b),
            Value::Number(n) => JsVal::Num(n.as_f64().unwrap_or(f64::NAN)),
            Value::String(s) => JsVal::Str(s.clone()),
            Value::Array(a) => JsVal::Arr(a.iter().map(JsVal::from_json).collect()),
            Value::Object(o) => {
                if o.len() <= 2
                    && let Some(Value::String(m)) = o.get("$")
                {
                    match m.as_str() {
                        "undefined" => return JsVal::Undefined,
                        "NaN" => return JsVal::Num(f64::NAN),
                        "Infinity" => return JsVal::Num(f64::INFINITY),
                        "-Infinity" => return JsVal::Num(f64::NEG_INFINITY),
                        "-0" => return JsVal::Num(-0.0),
                        "u16" => {
                            let units: Vec<u16> = o
                                .get("u")
                                .and_then(Value::as_array)
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|x| x.as_u64().map(|n| n as u16))
                                        .collect()
                                })
                                .unwrap_or_default();
                            return JsVal::Str(String::from_utf16_lossy(&units));
                        }
                        _ => {}
                    }
                }
                JsVal::Obj(
                    o.iter()
                        .map(|(k, x)| (k.clone(), JsVal::from_json(x)))
                        .collect(),
                )
            }
        }
    }

    /// Eigenschaft eines Objekts; alles andere (und fehlende Eigenschaften) liefert `undefined`.
    pub fn get(&self, key: &str) -> &JsVal {
        if let JsVal::Obj(o) = self
            && let Some((_, v)) = o.iter().find(|(k, _)| k == key)
        {
            return v;
        }
        &UNDEFINED
    }

    /// `v == null` (null oder undefined).
    pub fn is_nullish(&self) -> bool {
        matches!(self, JsVal::Undefined | JsVal::Null)
    }

    /// Elemente, falls Array (`Array.isArray(v) ? v : []`).
    pub fn as_array(&self) -> &[JsVal] {
        match self {
            JsVal::Arr(a) => a,
            _ => &[],
        }
    }
}

/// JS-Whitespace (`String.prototype.trim` / Regex `\s`).
pub fn is_js_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// `String.prototype.trim`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_ws)
}

/// `Number(string)` (StringToNumber).
pub fn string_to_number(s: &str) -> f64 {
    let t = js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let b = t.as_bytes();
    if b.len() > 2 && b[0] == b'0' {
        let radix = match b[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix != 0 {
            let mut v = 0.0f64;
            for c in t[2..].chars() {
                match c.to_digit(radix) {
                    Some(d) => v = v * f64::from(radix) + f64::from(d),
                    None => return f64::NAN,
                }
            }
            return v;
        }
    }
    // Dezimalliteral: [+-] (ziffern [. ziffern*] | . ziffern) [e [+-] ziffern]
    let mut i = 0;
    if b[i] == b'+' || b[i] == b'-' {
        i += 1;
    }
    let mut digits = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return f64::NAN;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let mut ed = 0;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            ed += 1;
        }
        if ed == 0 {
            return f64::NAN;
        }
    }
    if i != b.len() {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// `Number(v)`.
pub fn js_number(v: &JsVal) -> f64 {
    match v {
        JsVal::Undefined => f64::NAN,
        JsVal::Null => 0.0,
        JsVal::Bool(b) => f64::from(u8::from(*b)),
        JsVal::Num(n) => *n,
        JsVal::Str(s) => string_to_number(s),
        JsVal::Arr(_) => string_to_number(&js_string(v)),
        JsVal::Obj(_) => f64::NAN,
    }
}

/// `Number.prototype.toString()` (Basis 10).
pub fn num_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.is_infinite() {
        return if x > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    let a = x.abs();
    if (1e-6..1e21).contains(&a) {
        return format!("{x}");
    }
    // Exponentialform wie JS: 1.5e+21, 1e-7
    let s = format!("{x:e}");
    match s.split_once('e') {
        Some((m, e)) => {
            let sign = if e.starts_with('-') { "" } else { "+" };
            format!("{m}e{sign}{e}")
        }
        None => s,
    }
}

/// `String(v)`.
pub fn js_string(v: &JsVal) -> String {
    match v {
        JsVal::Undefined => "undefined".into(),
        JsVal::Null => "null".into(),
        JsVal::Bool(b) => b.to_string(),
        JsVal::Num(n) => num_to_string(*n),
        JsVal::Str(s) => s.clone(),
        JsVal::Arr(a) => a
            .iter()
            .map(|x| {
                if x.is_nullish() {
                    String::new()
                } else {
                    js_string(x)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        JsVal::Obj(_) => "[object Object]".into(),
    }
}

/// `String(v == null ? '' : v)`.
pub fn js_string_or_empty(v: &JsVal) -> String {
    if v.is_nullish() {
        String::new()
    } else {
        js_string(v)
    }
}

/// `Math.round` (rundet .5 Richtung +Unendlich).
pub fn round_js(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let r = x.floor();
    if x - r >= 0.5 { r + 1.0 } else { r }
}

fn quote_json(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn stringify_into(v: &JsVal, out: &mut String) {
    match v {
        JsVal::Undefined | JsVal::Null => out.push_str("null"),
        JsVal::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        JsVal::Num(n) => {
            if n.is_finite() {
                out.push_str(&num_to_string(*n));
            } else {
                out.push_str("null");
            }
        }
        JsVal::Str(s) => quote_json(s, out),
        JsVal::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                stringify_into(x, out);
            }
            out.push(']');
        }
        JsVal::Obj(o) => {
            out.push('{');
            let mut first = true;
            for (k, x) in o {
                if matches!(x, JsVal::Undefined) {
                    continue;
                }
                if !first {
                    out.push(',');
                }
                first = false;
                quote_json(k, out);
                out.push(':');
                stringify_into(x, out);
            }
            out.push('}');
        }
    }
}

/// `JSON.stringify(v)` (Schlüsselreihenfolge = Einfügereihenfolge, `undefined`-Eigenschaften entfallen).
pub fn json_stringify(v: &JsVal) -> String {
    let mut s = String::new();
    stringify_into(v, &mut s);
    s
}

/// `Object.keys` einer Eigenschaftsliste in JS-Reihenfolge: ganzzahlige Schlüssel aufsteigend,
/// danach die übrigen in Einfügereihenfolge.
pub fn object_entries_ordered(o: &[(String, JsVal)]) -> Vec<(&str, &JsVal)> {
    let is_index = |k: &str| {
        k == "0"
            || (!k.is_empty()
                && !k.starts_with('0')
                && k.bytes().all(|b| b.is_ascii_digit())
                && k.len() <= 10
                && k.parse::<u64>().is_ok_and(|n| n < 4_294_967_295))
    };
    let mut idx: Vec<(u64, &str, &JsVal)> = o
        .iter()
        .filter(|(k, _)| is_index(k))
        .map(|(k, v)| (k.parse::<u64>().unwrap_or(0), k.as_str(), v))
        .collect();
    idx.sort_by_key(|x| x.0);
    let mut out: Vec<(&str, &JsVal)> = idx.into_iter().map(|(_, k, v)| (k, v)).collect();
    out.extend(
        o.iter()
            .filter(|(k, _)| !is_index(k))
            .map(|(k, v)| (k.as_str(), v)),
    );
    out
}

//! Zahlenformat wie JavaScript (`Number.prototype.toFixed`, Dezimalkomma der Anzeige).
//!
//! `toFixed` rundet den exakten Dezimalwert des Doubles halb nach oben, Rusts `{:.N}` rundet
//! exakte Gleichstände zur geraden Ziffer (1250 ms ergibt in JS „1,3“, in Rust „1,2“). Darum eine
//! eigene Rundung auf der exakten Dezimalentwicklung.

/// `x.toFixed(digits)` für endliche Werte (`NaN`/`Infinity` wie JS als Text).
pub fn js_to_fixed(x: f64, digits: usize) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let neg = x < 0.0;
    // Exakte Entwicklung (Rust druckt Doubles bei hoher Genauigkeit exakt), dann halb nach oben.
    let full = format!("{:.*}", digits + 40, x.abs());
    let (int_part, frac_part) = full.split_once('.').unwrap_or((full.as_str(), ""));
    let mut digs: Vec<u8> = int_part
        .bytes()
        .chain(frac_part.bytes().take(digits))
        .collect();
    let round_up = frac_part.as_bytes().get(digits).is_some_and(|d| *d >= b'5');
    if round_up {
        let mut i = digs.len();
        loop {
            if i == 0 {
                digs.insert(0, b'1');
                break;
            }
            i -= 1;
            if digs[i] == b'9' {
                digs[i] = b'0';
            } else {
                digs[i] += 1;
                break;
            }
        }
    }
    let int_len = digs.len() - digits;
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.push_str(std::str::from_utf8(&digs[..int_len]).unwrap_or("0"));
    if digits > 0 {
        s.push('.');
        s.push_str(std::str::from_utf8(&digs[int_len..]).unwrap_or(""));
    }
    s
}

/// 1234 ms zu „1,2“ (Sekunden, ein Dezimalkomma).
pub fn sec(ms: f64) -> String {
    let v = if ms.is_finite() { ms.max(0.0) } else { 0.0 };
    js_to_fixed(v / 1000.0, 1).replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_fixed_rundet_gleichstand_nach_oben() {
        assert_eq!(js_to_fixed(1.25, 1), "1.3");
        assert_eq!(js_to_fixed(0.5, 0), "1");
        assert_eq!(js_to_fixed(2.5, 0), "3");
        assert_eq!(js_to_fixed(1.005, 2), "1.00"); // exakt 1.00499999...
        assert_eq!(js_to_fixed(9.95, 1), "9.9"); // exakt 9.949999...
        assert_eq!(js_to_fixed(99.96, 1), "100.0");
        assert_eq!(js_to_fixed(0.0, 2), "0.00");
        assert_eq!(js_to_fixed(-0.04, 1), "-0.0"); // wie JS: Vorzeichen bleibt
        assert_eq!(js_to_fixed(-0.0, 1), "0.0");
        assert_eq!(js_to_fixed(-1.25, 1), "-1.3");
    }

    #[test]
    fn sekunden_mit_komma() {
        assert_eq!(sec(1234.0), "1,2");
        assert_eq!(sec(1250.0), "1,3");
        assert_eq!(sec(-5.0), "0,0");
        assert_eq!(sec(f64::NAN), "0,0");
    }
}

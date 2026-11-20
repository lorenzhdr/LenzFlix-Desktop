//! Golden-Tests: vergleicht den Rust-Port bitgleich mit Vektoren, die `tools/golden/gen.mjs` aus
//! dem Original-JS-Planer erzeugt (siehe `tools/golden/README.md`).
//!
//! Fehlt der Ordner `tests/golden/`, schlägt der Test mit `LFX_GOLDEN_REQUIRED=1` fehl, sonst
//! wird er mit einer Meldung übersprungen. Zahlen werden als `f64` exakt verglichen
//! (`serde_json` mit `float_roundtrip`).

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use indexmap::IndexSet;
use lfx_tv_schedule::catalog::{Boxset, Catalog, Episode, Item, Movie, Series, normalize_catalog};
use lfx_tv_schedule::config::parse_channel_config;
use lfx_tv_schedule::fill::{
    Cand, DayBuilder, EntryType, Piece, PlaceOpts, PlanEntry, PremiereKind, ShortItem, age_allowed,
    closedown_day, fill_segment, pack_subset, slot_of_f64,
};
use lfx_tv_schedule::hash::{
    bitrev32, cmp_units, fmix32, fnv1a_units, key_hash_units, mulberry32, to_uint32,
};
use lfx_tv_schedule::jsval::{JsVal, js_number};
use lfx_tv_schedule::pool::{
    Entry, Stream, bucket, bucket_count, make_entries, rotation, rotation_with_borrow,
    stream_of_age,
};
use lfx_tv_schedule::rating::{max_age, rating_age};
use lfx_tv_schedule::scheduler::{PlanError, Scheduler};
use lfx_tv_schedule::time::{
    admit_day, broadcast_day_of, day_bounds, day_of_date, format_clock, format_day, offset_ms,
    parse_date, ticks_to_ms, wall_to_utc, week_of, week_start,
};
use serde_json::{Value, json};

thread_local! {
    /// Anzahl der Vergleiche des laufenden Tests.
    static COMPARED: Cell<usize> = const { Cell::new(0) };
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

/// `true`, wenn die Vektoren fehlen und der Test übersprungen werden soll.
fn skip() -> bool {
    if golden_dir().join("meta.json").exists() {
        return false;
    }
    assert!(
        std::env::var("LFX_GOLDEN_REQUIRED").as_deref() != Ok("1"),
        "Golden-Vektoren fehlen (tests/golden/); mit tools/golden/gen.mjs erzeugen"
    );
    eprintln!(
        "ÜBERSPRUNGEN: Golden-Vektoren fehlen (tests/golden/). Erzeugen: \
         LENZFLIX_WEB_DIR=<web-repo> node tools/golden/gen.mjs"
    );
    true
}

fn load(name: &str) -> Value {
    let p = golden_dir().join(name);
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{name}: {e}"));
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn count(n: usize) {
    COMPARED.with(|c| c.set(c.get() + n));
}

fn ok(name: &str, extra: &str) {
    eprintln!("{name} ok: {} Vergleiche{extra}", COMPARED.with(Cell::get));
}

// ------------------------------------------------------------------------------ Vergleich

/// Marker `{"$":"-0"}` wird zu 0 (Vorzeichen von -0 ist für den Planer ohne Bedeutung).
fn norm(v: &Value) -> Value {
    match v {
        Value::Object(o) if o.len() == 1 && o.get("$").and_then(Value::as_str) == Some("-0") => {
            json!(0.0)
        }
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), norm(x))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
        x => x.clone(),
    }
}

fn eq(exp: &Value, got: &Value, path: &str) -> Result<(), String> {
    match (exp, got) {
        (Value::Number(a), Value::Number(b)) => {
            if a.as_f64() == b.as_f64() {
                Ok(())
            } else {
                Err(format!("{path}: erwartet {a}, erhalten {b}"))
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Err(format!(
                    "{path}: Länge erwartet {}, erhalten {}",
                    a.len(),
                    b.len()
                ));
            }
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                eq(x, y, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        (Value::Object(a), Value::Object(b)) => {
            for (k, x) in a {
                match b.get(k) {
                    Some(y) => eq(x, y, &format!("{path}.{k}"))?,
                    None => return Err(format!("{path}.{k}: fehlt (erwartet {x})")),
                }
            }
            for k in b.keys() {
                if !a.contains_key(k) {
                    return Err(format!("{path}.{k}: unerwartet vorhanden ({})", b[k]));
                }
            }
            Ok(())
        }
        (a, b) if a == b => Ok(()),
        (a, b) => Err(format!("{path}: erwartet {a}, erhalten {b}")),
    }
}

fn check(exp: &Value, got: &Value, ctx: &str) {
    count(1);
    if let Err(e) = eq(&norm(exp), &norm(got), ctx) {
        let s = e.chars().take(600).collect::<String>();
        panic!("Abweichung in {s}");
    }
}

// ------------------------------------------------------------------------------ Umwandlung

/// Zahl als JSON mit Markern für NaN/Infinity/-0.
fn nv(x: f64) -> Value {
    if x.is_nan() {
        json!({"$": "NaN"})
    } else if x == f64::INFINITY {
        json!({"$": "Infinity"})
    } else if x == f64::NEG_INFINITY {
        json!({"$": "-Infinity"})
    } else if x == 0.0 && x.is_sign_negative() {
        json!({"$": "-0"})
    } else if x.fract() == 0.0 && x.abs() < 9e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

fn opt_nv(x: Option<f64>) -> Value {
    x.map_or(Value::Null, nv)
}

fn units_of(v: &Value) -> Vec<u16> {
    match v {
        Value::String(s) => s.encode_utf16().collect(),
        Value::Object(o) if o.get("$").and_then(Value::as_str) == Some("u16") => o["u"]
            .as_array()
            .expect("u16-Marker")
            .iter()
            .map(|x| x.as_u64().expect("Code-Unit") as u16)
            .collect(),
        x => panic!("keine Zeichenkette: {x}"),
    }
}

fn f(v: &Value) -> f64 {
    JsVal::from_json(v)
        .num()
        .unwrap_or_else(|| panic!("keine Zahl: {v}"))
}

trait NumExt {
    fn num(&self) -> Option<f64>;
}
impl NumExt for JsVal {
    fn num(&self) -> Option<f64> {
        match self {
            JsVal::Num(n) => Some(*n),
            _ => None,
        }
    }
}

fn i(v: &Value) -> i64 {
    f(v) as i64
}

fn age_of(v: &Value) -> Option<u8> {
    v.as_u64().map(|n| n as u8)
}

fn age_v(a: Option<u8>) -> Value {
    a.map_or(Value::Null, |x| json!(x))
}

fn entry_v(e: &PlanEntry) -> Value {
    let mut o = serde_json::Map::new();
    o.insert("start".into(), json!(e.start));
    o.insert("end".into(), json!(e.end));
    o.insert("type".into(), json!(e.kind.as_str()));
    if let Some(r) = &e.reason {
        o.insert("reason".into(), json!(r));
    }
    if let Some(x) = &e.item_id {
        o.insert("itemId".into(), json!(x));
    }
    if let Some(x) = &e.series_id {
        o.insert("seriesId".into(), json!(x));
    }
    if e.rerun {
        o.insert("rerun".into(), json!(true));
    }
    if let Some(p) = e.premiere {
        o.insert("premiere".into(), json!(p.as_str()));
    }
    if let Some(x) = &e.theme {
        o.insert("theme".into(), json!(x));
    }
    if let Some(a) = e.age {
        o.insert("age".into(), json!(a));
    }
    if let Some(x) = &e.label {
        o.insert("label".into(), json!(x));
    }
    Value::Object(o)
}

fn entries_v(l: &[PlanEntry]) -> Value {
    Value::Array(l.iter().map(entry_v).collect())
}

fn ids_of<T>(l: &[Entry<T>]) -> Value {
    Value::Array(l.iter().map(|e| json!(e.id)).collect())
}

// ------------------------------------------------------------------------------ Primitive

#[test]
fn hash_vectors() {
    if skip() {
        return;
    }
    let p = load("primitives.json");
    for c in p["fnv1a"].as_array().unwrap() {
        check(&c["out"], &json!(fnv1a_units(&units_of(&c["in"]))), "fnv1a");
    }
    for c in p["fmix32"].as_array().unwrap() {
        check(&c["out"], &json!(fmix32(i(&c["in"]) as u32)), "fmix32");
    }
    for c in p["bitrev32"].as_array().unwrap() {
        check(&c["out"], &json!(bitrev32(i(&c["in"]) as u32)), "bitrev32");
    }
    for c in p["mulberry32"].as_array().unwrap() {
        let mut g = mulberry32(f(&c["seed"]));
        let got: Vec<Value> = (0..24).map(|_| nv(g.next())).collect();
        check(
            &c["out"],
            &Value::Array(got),
            &format!("mulberry32 {}", c["seed"]),
        );
    }
    for c in p["keyHash"].as_array().unwrap() {
        let got = key_hash_units(&units_of(&c["key"]), &units_of(&c["id"]));
        check(&c["out"], &json!(got), "keyHash");
    }
    for c in p["cmpStr"].as_array().unwrap() {
        let got = match cmp_units(&units_of(&c["a"]), &units_of(&c["b"])) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        check(
            &c["out"],
            &json!(got),
            &format!("cmpStr {} {}", c["a"], c["b"]),
        );
    }
    assert_eq!(to_uint32(4_294_967_296.0 + 5.0), 5);
    ok("hash_vectors", "");
}

#[test]
fn time_vectors() {
    if skip() {
        return;
    }
    let p = load("primitives.json");
    for c in p["offsetMs"].as_array().unwrap() {
        check(
            &c["out"],
            &json!(offset_ms(i(&c["in"]))),
            &format!("offsetMs {}", c["in"]),
        );
    }
    for k in ["broadcastDayOf", "broadcastDayOfEdges"] {
        for c in p[k].as_array().unwrap() {
            check(
                &c["out"],
                &json!(broadcast_day_of(i(&c["in"]))),
                &format!("{k} {}", c["in"]),
            );
        }
    }
    for c in p["formatClock"].as_array().unwrap() {
        check(&c["out"], &json!(format_clock(i(&c["in"]))), "formatClock");
    }
    for c in p["admitDay"].as_array().unwrap() {
        check(&c["out"], &json!(admit_day(i(&c["in"]))), "admitDay");
    }
    for c in p["wallToUtc"].as_array().unwrap() {
        check(
            &c["out"],
            &json!(wall_to_utc(i(&c["D"]), i(&c["min"]))),
            "wallToUtc",
        );
    }
    for c in p["dayBounds"].as_array().unwrap() {
        let b = day_bounds(i(&c["D"]));
        let got = json!({"start": b.start, "end": b.end, "a2015": b.a2015, "a2200": b.a2200, "a2300": b.a2300});
        check(&c["out"], &got, &format!("dayBounds {}", c["D"]));
    }
    for c in p["formatDay"].as_array().unwrap() {
        let l = format_day(i(&c["D"]));
        check(
            &c["out"],
            &json!({"weekday": l.weekday, "long": l.long, "date": l.date}),
            &format!("formatDay {}", c["D"]),
        );
    }
    for c in p["week"].as_array().unwrap() {
        let d = i(&c["D"]);
        check(&c["weekOf"], &json!(week_of(d)), "weekOf");
        if let Some(ws) = c.get("weekStartOfWeek") {
            check(ws, &json!(week_start(week_of(d))), "weekStart(weekOf)");
        }
    }
    for c in p["weekStart"].as_array().unwrap() {
        check(&c["out"], &json!(week_start(i(&c["W"]))), "weekStart");
    }
    for c in p["dayOfDate"].as_array().unwrap() {
        let got = day_of_date(f(&c["y"]), f(&c["m"]), f(&c["d"]));
        check(
            &c["out"],
            &json!(got),
            &format!("dayOfDate {} {} {}", c["y"], c["m"], c["d"]),
        );
    }
    for c in p["parseDate"].as_array().unwrap() {
        let got = opt_nv(parse_date(&JsVal::from_json(&c["in"])));
        check(&c["out"], &got, &format!("parseDate {}", c["in"]));
    }
    for c in p["ticksToMs"].as_array().unwrap() {
        let got = nv(ticks_to_ms(&JsVal::from_json(&c["in"])));
        check(&c["out"], &got, &format!("ticksToMs {}", c["in"]));
    }
    ok("time_vectors", "");
}

#[test]
fn rating_catalog_helper_vectors() {
    if skip() {
        return;
    }
    let p = load("primitives.json");
    for c in p["ratingAge"].as_array().unwrap() {
        let got = age_v(rating_age(&JsVal::from_json(&c["in"])));
        check(&c["out"], &got, &format!("ratingAge {}", c["in"]));
    }
    for c in p["maxAge"].as_array().unwrap() {
        let got = age_v(max_age(age_of(&c["a"]), age_of(&c["b"])));
        check(&c["out"], &got, "maxAge");
    }
    for c in p["normGenre"].as_array().unwrap() {
        let got = json!(lfx_tv_schedule::catalog::norm_genre(&JsVal::from_json(
            &c["in"]
        )));
        check(&c["out"], &got, &format!("normGenre {}", c["in"]));
    }
    for c in p["lowerMedian"].as_array().unwrap() {
        let arr: Vec<i64> = c["in"].as_array().unwrap().iter().map(i).collect();
        check(
            &c["out"],
            &json!(lfx_tv_schedule::catalog::lower_median(&arr)),
            "lowerMedian",
        );
    }
    for c in p["parseChannelConfigFmt"].as_array().unwrap() {
        let m = parse_channel_config(&JsVal::from_json(&c["in"]));
        let got: serde_json::Map<String, Value> = m
            .into_iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect();
        check(
            &c["out"],
            &Value::Object(got),
            &format!("parseChannelConfig {}", c["in"]),
        );
    }
    ok("rating_catalog_helper_vectors", "");
}

#[test]
fn fill_primitive_vectors() {
    if skip() {
        return;
    }
    let p = load("primitives.json");
    for c in p["slotOf"].as_array().unwrap() {
        check(&c["out"], &nv(slot_of_f64(f(&c["in"]))), "slotOf");
    }
    for c in p["ageAllowed"].as_array().unwrap() {
        let b = day_bounds(i(&c["D"]));
        let got = age_allowed(age_of(&c["age"]), i(&c["start"]), &b);
        check(&c["out"], &json!(got), "ageAllowed");
    }
    for c in p["packSubset"].as_array().unwrap() {
        let sizes: Vec<i64> = c["sizes"].as_array().unwrap().iter().map(i).collect();
        let got: Vec<usize> = pack_subset(&sizes, i(&c["cap"]));
        check(
            &c["out"],
            &json!(got),
            &format!("packSubset {} {}", c["sizes"], c["cap"]),
        );
    }
    for c in p["closedownDay"].as_array().unwrap() {
        let reason = c.get("reason").and_then(Value::as_str).unwrap_or("empty");
        let got = entries_v(&closedown_day(i(&c["D"]), reason));
        check(&c["out"], &got, "closedownDay");
    }
    ok("fill_primitive_vectors", "");
}

#[test]
fn pool_vectors() {
    if skip() {
        return;
    }
    let p = load("primitives-pool.json");
    for c in p["streamOfAge"].as_array().unwrap() {
        let got = match stream_of_age(age_of(&c["in"])) {
            Stream::A => "A",
            Stream::B => "B",
        };
        check(&c["out"], &json!(got), "streamOfAge");
    }
    for c in p["bucketCount"].as_array().unwrap() {
        let got = bucket_count(i(&c["poolMs"]), i(&c["capMs"]));
        check(
            &c["out"],
            &json!(got),
            &format!("bucketCount {} {}", c["poolMs"], c["capMs"]),
        );
    }
    check(
        &p["capMs"],
        &json!({"A": lfx_tv_schedule::pool::CAP_A, "B": lfx_tv_schedule::pool::CAP_B}),
        "CAP_MS",
    );
    let ent = |e: &[Entry<(String, i64)>]| -> Value {
        Value::Array(
            e.iter()
                .map(|x| json!({"id": x.id, "h": x.h, "br": x.br, "ms": x.ms}))
                .collect(),
        )
    };
    for blk in p["entries"].as_array().unwrap() {
        let key = blk["key"].as_str().unwrap();
        let items: Vec<(String, i64)> = blk["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| (x["id"].as_str().unwrap().to_string(), i(&x["durMs"])))
            .collect();
        let sorted = make_entries(key, &items, |t| t.0.clone(), |t| t.1);
        check(&blk["sorted"], &ent(&sorted), &format!("makeEntries {key}"));
        for cs in blk["cases"].as_array().unwrap() {
            let (c, d) = (i(&cs["C"]), i(&cs["D"]));
            let ctx = format!("pool {key} C={c} D={d}");
            check(&cs["bucket"], &ids_of(&bucket(&sorted, c, d)), &ctx);
            check(&cs["rotation"], &ids_of(&rotation(&sorted, c, d)), &ctx);
            let (home, borrow) = rotation_with_borrow(&sorted, c, d);
            check(
                &cs["borrow"],
                &json!({"home": ids_of(&home), "borrow": ids_of(&borrow)}),
                &ctx,
            );
        }
    }
    let m = &p["entriesMsOf"];
    let items: Vec<(String, i64)> = m["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| (x["id"].as_str().unwrap().to_string(), i(&x["durMs"])))
        .collect();
    let sorted = make_entries("x", &items, |t| t.0.clone(), |t| t.0.len() as i64 * 1000);
    check(&m["out"], &ent(&sorted), "makeEntries msOf");
    ok("pool_vectors", "");
}

// ------------------------------------------------------------------------------ Builder

fn piece_of(v: &Value) -> Piece {
    let mut p = Piece::new(
        v["itemId"].as_str().unwrap(),
        i(&v["durMs"]),
        age_of(&v["age"]),
    );
    p.series_id = v
        .get("seriesId")
        .and_then(Value::as_str)
        .map(str::to_string);
    p.rerun = v.get("rerun").and_then(Value::as_bool).unwrap_or(false);
    p.premiere = v.get("premiere").and_then(Value::as_str).map(|s| match s {
        "film" => PremiereKind::Film,
        _ => PremiereKind::Episode,
    });
    p.theme = v.get("theme").and_then(Value::as_str).map(str::to_string);
    p.label = v.get("label").and_then(Value::as_str).map(str::to_string);
    p
}

fn shorts_of(v: &Value) -> Rc<Vec<ShortItem>> {
    Rc::new(
        v.as_array()
            .unwrap()
            .iter()
            .map(|s| ShortItem {
                id: s["id"].as_str().unwrap().to_string(),
                dur_ms: i(&s["durMs"]),
                age: age_of(&s["age"]),
                series_id: s
                    .get("seriesId")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
            .collect(),
    )
}

fn opts_of(v: Option<&Value>) -> Option<PlaceOpts> {
    let o = v?.as_object()?;
    Some(PlaceOpts {
        limit: o.get("limit").map(i),
        kind: o.get("type").and_then(Value::as_str).map(|t| match t {
            "fill" => EntryType::Fill,
            _ => EntryType::Program,
        }),
    })
}

#[test]
fn builder_vectors() {
    if skip() {
        return;
    }
    let b = load("builder.json");
    for (n, sc) in b["scenarios"].as_array().unwrap().iter().enumerate() {
        let used: IndexSet<String> = sc["usedInit"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        let key = sc.get("key").and_then(Value::as_str);
        let mut bld = DayBuilder::new(i(&sc["D"]), None, used, shorts_of(&sc["shorts"]), key);
        for (k, op) in sc["ops"].as_array().unwrap().iter().enumerate() {
            let res = match op["op"].as_str().unwrap() {
                "place" => Value::Bool(bld.place(&piece_of(&op["piece"]), opts_of(op.get("opts")))),
                "canPlace" => {
                    Value::Bool(bld.can_place(&piece_of(&op["piece"]), opts_of(op.get("opts"))))
                }
                "placeBlock" => {
                    let ps: Vec<Piece> = op["pieces"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(piece_of)
                        .collect();
                    Value::Bool(bld.place_block(&ps, opts_of(op.get("opts"))))
                }
                "fillUntil" => {
                    bld.fill_until(i(&op["until"]));
                    Value::Null
                }
                "closeRest" => {
                    bld.close_rest(op.get("reason").and_then(Value::as_str).unwrap_or("end"));
                    Value::Null
                }
                x => panic!("unbekannte Operation {x}"),
            };
            check(
                &sc["results"][k],
                &json!([res, bld.cursor()]),
                &format!("builder {n} op {k}"),
            );
        }
        let fin = bld.finish();
        check(
            &sc["finish"],
            &entries_v(&fin),
            &format!("builder {n} finish"),
        );
        let used_after: Vec<&String> = bld.used().iter().collect();
        check(
            &sc["usedAfter"],
            &json!(used_after),
            &format!("builder {n} usedAfter"),
        );
    }
    for (n, sg) in b["segs"].as_array().unwrap().iter().enumerate() {
        let mut bld = DayBuilder::new(
            i(&sg["D"]),
            None,
            IndexSet::new(),
            shorts_of(&sg["shorts"]),
            Some("3"),
        );
        let cands: Vec<Cand> = sg["cands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| Cand {
                id: c["id"].as_str().unwrap().to_string(),
                movie: false,
                size: i(&c["size"]),
                pieces: c["pieces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(piece_of)
                    .collect(),
            })
            .collect();
        if let Some(u) = sg["pre"].get("fillUntil") {
            bld.fill_until(i(u));
        }
        fill_segment(
            &mut bld,
            &cands,
            i(&sg["limit"]),
            sg["strict"].as_bool().unwrap(),
        );
        check(
            &sg["finish"],
            &entries_v(&bld.finish()),
            &format!("fillSegment {n}"),
        );
    }
    ok("builder_vectors", "");
}

// ------------------------------------------------------------------------------ Katalog

fn raw_of(name: &str) -> (JsVal, Contents) {
    let v = load(&format!("catalog-{name}.raw.json"));
    let raw = JsVal::from_json(&v["raw"]);
    // None = Fehler (wirft), Some(None) = null, Some(Some(liste))
    let mut contents = HashMap::new();
    for (k, x) in v["boxsetContents"].as_object().unwrap() {
        let val = match x {
            Value::Null => Some(None),
            Value::Object(o) if o.get("$").and_then(Value::as_str) == Some("throw") => None,
            Value::Array(a) => Some(Some(
                a.iter().map(|s| s.as_str().unwrap().to_string()).collect(),
            )),
            o => panic!("unbekannter Sammlungs-Inhalt {o}"),
        };
        contents.insert(k.clone(), val);
    }
    (raw, contents)
}

fn strs(v: &[String]) -> Value {
    json!(v)
}

fn movie_v(m: &Movie) -> Value {
    json!({
        "id": m.id, "kind": "movie", "title": m.title, "durMs": m.dur_ms, "genres": strs(&m.genres),
        "year": m.year, "premiere": opt_nv(m.premiere), "addedAt": nv(m.added_at),
        "age": age_v(m.age), "trailer": m.trailer,
    })
}

fn episode_v(e: &Episode) -> Value {
    json!({
        "id": e.id, "kind": "episode", "title": e.title, "durMs": e.dur_ms,
        "seriesId": e.series_id, "seriesTitle": e.series_title, "season": nv(e.season),
        "episode": nv(e.episode), "premiere": nv(e.premiere), "addedAt": nv(e.added_at),
        "age": age_v(e.age), "genres": strs(&e.genres), "year": e.year,
    })
}

fn series_v(s: &Series) -> Value {
    json!({
        "id": s.id, "kind": "series", "title": s.title, "genres": strs(&s.genres), "year": s.year,
        "addedAt": nv(s.added_at), "age": age_v(s.age), "ageMax": age_v(s.age_max),
        "eps": s.eps.iter().map(|e| episode_v(e)).collect::<Vec<_>>(),
        "epAddedSorted": s.ep_added_sorted.iter().map(|x| nv(*x)).collect::<Vec<_>>(),
        "firstEpAdded": nv(s.first_ep_added), "totalMs": s.total_ms, "durMs": s.dur_ms,
    })
}

fn boxset_v(b: &Boxset) -> Value {
    json!({"id": b.id, "name": b.name, "childCount": nv(b.child_count)})
}

fn compare_catalog(name: &str, cat: &Catalog) {
    let n = load(&format!("catalog-{name}.norm.json"));
    check(
        &n["movies"],
        &Value::Array(cat.movies.iter().map(|m| movie_v(m)).collect()),
        &format!("{name}.movies"),
    );
    check(
        &n["series"],
        &Value::Array(cat.series.iter().map(|s| series_v(s)).collect()),
        &format!("{name}.series"),
    );
    let order: Vec<Value> = cat
        .items
        .iter()
        .map(|(id, it)| {
            let kind = match it {
                Item::Movie(_) => "movie",
                Item::Episode(_) => "episode",
                Item::Series(_) => "series",
            };
            json!([id, kind])
        })
        .collect();
    check(
        &n["itemsOrder"],
        &Value::Array(order),
        &format!("{name}.itemsOrder"),
    );
    check(
        &n["boxsets"],
        &Value::Array(cat.boxsets.iter().map(boxset_v).collect()),
        &format!("{name}.boxsets"),
    );
    let genres: Vec<Value> = cat
        .genres
        .iter()
        .map(|g| json!({"label": g.label, "count": g.count, "norm": g.norm}))
        .collect();
    check(
        &n["genres"],
        &Value::Array(genres),
        &format!("{name}.genres"),
    );
    for p in n["genreLabel"].as_array().unwrap() {
        let got = json!(cat.genre_label(p["in"].as_str().unwrap()));
        check(&p["out"], &got, &format!("{name}.genreLabel {}", p["in"]));
    }
    check(
        &n["loadedAt"],
        &nv(cat.loaded_at),
        &format!("{name}.loadedAt"),
    );
    let sched = Scheduler::new(Arc::new(cat.clone()), &JsVal::Undefined);
    let api: Vec<Value> = sched
        .genres()
        .into_iter()
        .map(|(l, c)| json!({"label": l, "count": c}))
        .collect();
    check(
        &n["genresApi"],
        &Value::Array(api),
        &format!("{name}.genresApi"),
    );
}

fn catalog_test(name: &str) {
    if skip() {
        return;
    }
    let (raw, _) = raw_of(name);
    let cat = normalize_catalog(&raw);
    compare_catalog(name, &cat);
    ok(&format!("catalog {name}"), "");
}

#[test]
fn catalog_thin() {
    catalog_test("thin");
}
#[test]
fn catalog_normal() {
    catalog_test("normal");
}
#[test]
fn catalog_large() {
    catalog_test("large");
}
#[test]
fn catalog_edge() {
    catalog_test("edge");
}

// ------------------------------------------------------------------------------ Pläne

type Contents = HashMap<String, Option<Option<Vec<String>>>>;

/// Lädt die Sammlungs-Inhalte wie der Mock des Generators; zeichnet die Aufrufe auf.
fn prepare(s: &mut Scheduler, days: &[i64], contents: &Contents) -> Vec<String> {
    let mut calls = Vec::new();
    s.prepare(days, |id| {
        calls.push(id.to_string());
        match contents.get(id) {
            None => Ok(Some(Vec::new())),
            Some(None) => Err(()),
            Some(Some(v)) => Ok(v.clone()),
        }
    });
    calls
}

fn plan_files(catalog: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(golden_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with(&format!("plans-{catalog}-")) && n.ends_with(".json"))
        .collect();
    v.sort();
    v
}

fn channels_v(l: &[lfx_tv_schedule::scheduler::Channel]) -> Value {
    Value::Array(
        l.iter()
            .map(|c| {
                let mut o = json!({"no": c.no, "key": c.key, "name": c.name, "kind": c.kind});
                if let Some(g) = &c.genre_label {
                    o["genreLabel"] = json!(g);
                }
                o
            })
            .collect(),
    )
}

fn plans_test(catalog: &str) {
    if skip() {
        return;
    }
    let (raw, contents) = raw_of(catalog);
    let cat = Arc::new(normalize_catalog(&raw));
    let files = plan_files(catalog);
    assert!(!files.is_empty(), "keine Plan-Dateien für {catalog}");
    let mut total_entries = 0usize;
    for file in &files {
        let v = load(file);
        let cfg = JsVal::from_json(&v["config"]);
        let days: Vec<i64> = v["days"].as_array().unwrap().iter().map(i).collect();
        let mut s = Scheduler::new(cat.clone(), &cfg);
        if v["prepared"].as_bool().unwrap() {
            let calls = prepare(&mut s, &days, &contents);
            check(
                &v["prepareCalls"],
                &json!(calls),
                &format!("{file} prepareCalls"),
            );
        }
        for d in &days {
            let ch = s.channels(*d);
            check(
                &v["channels"][d.to_string()],
                &channels_v(&ch),
                &format!("{file} channels {d}"),
            );
        }
        for (no, per_day) in v["plans"].as_object().unwrap() {
            let no: i64 = no.parse().unwrap();
            for d in &days {
                let plan = s.plan_day(no, *d).expect("gültiger Plan");
                let exp = &per_day[d.to_string()];
                total_entries += plan.len();
                check(
                    exp,
                    &entries_v(&plan),
                    &format!("{file} Sender {no} Tag {d}"),
                );
            }
        }
    }
    ok(
        &format!("plans {catalog}"),
        &format!(" ({} Dateien, {total_entries} Planeinträge)", files.len()),
    );
}

#[test]
fn plans_thin() {
    plans_test("thin");
}
#[test]
fn plans_normal() {
    plans_test("normal");
}
#[test]
fn plans_large() {
    plans_test("large");
}
#[test]
fn plans_edge() {
    plans_test("edge");
}

// ------------------------------------------------------------------------------ jetzt/als Nächstes

fn now_v(n: &lfx_tv_schedule::scheduler::NowOn) -> Value {
    json!({"entry": entry_v(&n.entry), "offsetMs": n.offset_ms, "D": n.d})
}

/// `planDay(no, D)` mit JS-Eingaben (Fehler wie `RangeError`).
fn plan_day_js(s: &mut Scheduler, no: &JsVal, d: &JsVal) -> Result<Rc<Vec<PlanEntry>>, PlanError> {
    let nf = js_number(no);
    if !nf.is_finite() || nf.fract() != 0.0 {
        return Err(PlanError);
    }
    match d {
        JsVal::Num(x) if x.is_finite() && x.fract() == 0.0 => s.plan_day(nf as i64, *x as i64),
        _ => Err(PlanError),
    }
}

fn now_test(name: &str) {
    if skip() {
        return;
    }
    let (raw, contents) = raw_of(name);
    let cat = Arc::new(normalize_catalog(&raw));
    let meta = load("meta.json");
    let w0 = &meta["windows"]["w0"];
    let days: Vec<i64> = (0..i(&w0["days"])).map(|k| i(&w0["fromD"]) + k).collect();
    let v = load(&format!("now-{name}.json"));
    let mut s = Scheduler::new(cat.clone(), &JsVal::Undefined);
    prepare(&mut s, &days, &contents);
    for c in v["cases"].as_array().unwrap() {
        let (no, t) = (i(&c["no"]), i(&c["t"]));
        let ctx = format!("now-{name} no={no} t={t}");
        check(&c["nowOn"], &now_v(&s.now_on(no, t).unwrap()), &ctx);
        check(
            &c["up3"],
            &entries_v(&s.upcoming(no, t, 3).unwrap()),
            &format!("{ctx} up3"),
        );
        if let Some(u) = c.get("up12") {
            check(
                u,
                &entries_v(&s.upcoming(no, t, 12).unwrap()),
                &format!("{ctx} up12"),
            );
        }
    }
    for c in v["dst"].as_array().unwrap() {
        let (no, t) = (i(&c["no"]), i(&c["t"]));
        let mut fresh = Scheduler::new(cat.clone(), &JsVal::Undefined);
        let ctx = format!("now-{name} dst no={no} t={t}");
        check(&c["nowOn"], &now_v(&fresh.now_on(no, t).unwrap()), &ctx);
        check(
            &c["up3"],
            &entries_v(&fresh.upcoming(no, t, 3).unwrap()),
            &format!("{ctx} up3"),
        );
    }
    for c in v["errors"].as_array().unwrap() {
        let mut fresh = Scheduler::new(cat.clone(), &JsVal::Undefined);
        let res = plan_day_js(
            &mut fresh,
            &JsVal::from_json(&c["no"]),
            &JsVal::from_json(&c["D"]),
        );
        let got = match res {
            Ok(p) => json!({"ok": true, "firstType": p[0].kind.as_str()}),
            Err(e) => json!({"ok": false, "name": "RangeError", "message": e.to_string()}),
        };
        check(
            &c["res"],
            &got,
            &format!("now-{name} errors no={} D={}", c["no"], c["D"]),
        );
    }
    if let Some(l) = v.get("longUpcoming") {
        let mut sc = Scheduler::new(cat.clone(), &JsVal::Undefined);
        for c in l.as_array().unwrap() {
            let (no, t, n) = (i(&c["no"]), i(&c["t"]), i(&c["n"]) as usize);
            let got = entries_v(&sc.upcoming(no, t, n).unwrap());
            check(
                &c["out"],
                &got,
                &format!("now-{name} longUpcoming no={no} t={t}"),
            );
        }
    }
    ok(&format!("now-{name}"), "");
}

#[test]
fn now_thin() {
    now_test("thin");
}
#[test]
fn now_normal() {
    now_test("normal");
}
#[test]
fn now_large() {
    now_test("large");
}
#[test]
fn now_edge() {
    now_test("edge");
}

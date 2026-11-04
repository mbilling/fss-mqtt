//! Exporting a namespace: every topic below a node (a branch, or a whole
//! connection) as a model of what's there, in JSON and in CSV. For each
//! topic its path, its unified namespace levels, format and content type,
//! messages, whether retained, when last, how often, its latest value
//! (with unit and quality), its JSON fields' types, any flag, and a text
//! example.

use web_time::SystemTime;

use crate::fmt;
use crate::store::{Message, Store};

/// An export: its file name (without extension), the JSON and CSV, and how
/// many topics it has.
pub struct Export {
    pub name: String,
    pub json: String,
    pub csv: String,
    pub topics: usize,
}

const COLUMNS: [&str; 14] = [
    "topic",
    "levels",
    "format",
    "content_type",
    "messages",
    "retained",
    "last",
    "every_s",
    "value",
    "unit",
    "quality",
    "fields",
    "flag",
    "example",
];

/// Every topic with messages at or below node `root`, in the tree's order.
pub fn namespace(st: &Store, root: usize, now: SystemTime) -> Export {
    let node = &st.nodes[root];
    let conn = &st.conns[node.conn];
    let mut rows: Vec<[String; 14]> = Vec::new();
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        let t = &st.nodes[i];
        stack.extend(t.children().iter().rev());
        let Some(m) = t.latest() else { continue };
        let r = reading(m);
        let levels: Vec<String> = crate::uns::levels(&conn.uns, &t.path)
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let w = t.watch.as_deref();
        let fields: Vec<String> = w.map_or(vec![], |w| w.fields().map(|(n, k)| format!("{n}:{k}")).collect());
        let example = if crate::fmt::is_text(&m.payload) {
            fmt::one_line(&m.payload, 200)
        } else {
            String::new()
        };
        rows.push([
            t.path.clone(),
            levels.join(";"),
            crate::pretty::detect(m).label().to_string(),
            m.props.content_type.clone().unwrap_or_default(),
            t.msg_count.to_string(),
            m.retain.to_string(),
            iso(m.time),
            w.and_then(|w| w.every()).map(fmt::number).unwrap_or_default(),
            if r.plain { r.value } else { String::new() },
            r.unit.unwrap_or_default(),
            r.quality.unwrap_or_default(),
            fields.join(";"),
            w.and_then(|w| w.flag(now))
                .map(|f| format!("{}: {}", f.kind.name(), f.what))
                .unwrap_or_default(),
            example,
        ]);
    }

    let mut csv = COLUMNS.join(",") + "\n";
    for r in &rows {
        csv += &r.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(",");
        csv.push('\n');
    }

    let mut json = String::from("{\n");
    json += &format!("  \"exported\": {},\n", quote(&iso(now)));
    json += &format!("  \"connection\": {},\n", quote(&conn.name));
    json += &format!("  \"url\": {},\n", quote(&conn.url));
    if !conn.uns.is_empty() {
        let l: Vec<String> = conn.uns.iter().map(|l| quote(l)).collect();
        json += &format!("  \"uns\": [{}],\n", l.join(", "));
    }
    json += &format!("  \"root\": {},\n", quote(&node.path));
    json += "  \"topics\": [";
    for (i, r) in rows.iter().enumerate() {
        json += if i == 0 { "\n    {" } else { ",\n    {" };
        let mut first = true;
        for (k, v) in COLUMNS.iter().zip(r) {
            // Numbers and booleans as such; what's empty, left out.
            let v = match *k {
                _ if v.is_empty() => continue,
                "messages" | "every_s" | "retained" => v.clone(),
                "levels" => {
                    let kv: Vec<String> = v
                        .split(';')
                        .filter_map(|p| p.split_once('='))
                        .map(|(a, b)| format!("{}: {}", quote(a), quote(b)))
                        .collect();
                    format!("{{{}}}", kv.join(", "))
                }
                "fields" => {
                    let kv: Vec<String> = v
                        .split(';')
                        .filter_map(|p| p.rsplit_once(':'))
                        .map(|(a, b)| format!("{}: {}", quote(a), quote(b)))
                        .collect();
                    format!("{{{}}}", kv.join(", "))
                }
                _ => quote(v),
            };
            json += &format!("{}{}: {v}", if first { "" } else { ", " }, quote(k));
            first = false;
        }
        json += "}";
    }
    json += "\n  ]\n}\n";

    let place: String = node
        .path
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stamp = iso(now).replace([':', '-'], "");
    let name = match place.trim_matches('_') {
        "" => format!("fss-mqtt-{}-{stamp}", safe(&conn.name)),
        p => format!("fss-mqtt-{}-{p}-{stamp}", safe(&conn.name)),
    };
    Export {
        name,
        json,
        csv,
        topics: rows.len(),
    }
}

fn safe(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect()
}

/// 2026-11-04T21:30:05Z
fn iso(t: SystemTime) -> String {
    let secs = t.duration_since(web_time::UNIX_EPOCH).unwrap_or_default().as_secs();
    crate::values::timestamp(secs as i64, 0, true).replace(' ', "T")
}

/// A JSON string.
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '\t' => out += "\\t",
            c if (c as u32) < 0x20 => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A CSV cell: quoted when it holds a comma, a quote or a line break.
fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// A topic's latest value, as the values table and an export show it.
#[derive(Default)]
pub struct Reading {
    pub value: String,
    /// A value of its own (not a summary of the payload): right-aligned.
    pub plain: bool,
    pub unit: Option<String>,
    pub quality: Option<String>,
}

/// What a message says, in a word or two: a `{"value", "unit",
/// "quality"}` object's (as a unified namespace's tags often are), a short
/// text or number, the first numbers a JSON payload carries, or else what
/// it is and its size.
pub fn reading(m: &Message) -> Reading {
    let quality = m
        .props
        .user
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("quality"))
        .map(|(_, v)| v.clone());
    let text = |j: &crate::json::Json| match j {
        crate::json::Json::Str(s) => Some(s.clone()),
        crate::json::Json::Num(n) => Some(fmt::number(*n)),
        crate::json::Json::Bool(b) => Some(b.to_string()),
        _ => None,
    };
    if m.payload.len() == m.size
        && let Ok(j @ crate::json::Json::Obj(_)) = crate::json::Json::parse(&m.payload, 8)
    {
        let first = |keys: &[&str]| keys.iter().find_map(|k| j.get(k)).and_then(text);
        if let Some(value) = first(&["value", "val", "v"]) {
            return Reading {
                value,
                plain: true,
                unit: first(&["unit", "units", "eng_unit", "engUnit", "uom"]),
                quality: first(&["quality", "q"]).or(quality),
            };
        }
        let nums = crate::chart::fields(m);
        if !nums.is_empty() {
            let shown: Vec<String> = nums
                .iter()
                .take(3)
                .map(|(k, v)| format!("{k} {}", fmt::number(*v)))
                .collect();
            return Reading {
                value: shown.join(" · "),
                quality,
                ..Default::default()
            };
        }
    }
    if m.size <= 32 && fmt::is_text(&m.payload) {
        return Reading {
            value: String::from_utf8_lossy(&m.payload).trim().to_string(),
            plain: true,
            quality,
            ..Default::default()
        };
    }
    Reading {
        value: format!("{} · {}", crate::pretty::detect(m).label(), fmt::human_bytes(m.size)),
        quality,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Props;

    #[test]
    fn a_branch_exported_as_json_and_csv() {
        let mut st = Store::new(10, 64 * 1024);
        let c = st.add_conn("acme", "mqtt://x:1883", None, vec![]);
        st.set_uns(c, ["enterprise", "site", "area"].map(String::from).to_vec());
        for i in 0..5 {
            let p = format!("{{\"value\":{},\"unit\":\"bpm\",\"quality\":\"Good\"}}", 400 + i);
            st.add_decoded(
                c,
                "acme/oslo/packaging/speed",
                p.as_bytes(),
                0,
                false,
                Props::default(),
                None,
            );
        }
        st.add_decoded(
            c,
            "acme/oslo/packaging/note",
            b"hello, \"world\"",
            1,
            true,
            Props::default(),
            None,
        );
        st.add_decoded(c, "acme/bergen", b"7", 0, false, Props::default(), None);
        st.sort_children();
        let oslo = st.find(c, "acme/oslo").unwrap();
        let e = namespace(&st, oslo, SystemTime::now());
        assert_eq!(e.topics, 2);
        assert!(e.name.starts_with("fss-mqtt-acme-acme_oslo-"), "{}", e.name);

        // CSV: a header, then a row a topic; a cell with a comma or a quote quoted.
        let lines: Vec<&str> = e.csv.lines().collect();
        assert_eq!(lines[0], COLUMNS.join(","));
        assert!(
            lines[1].starts_with("acme/oslo/packaging/note,enterprise=acme;site=oslo;area=packaging,text,,1,true,"),
            "{}",
            lines[1]
        );
        assert!(lines[1].ends_with(",\"hello, \"\"world\"\"\""), "{}", lines[1]);
        assert!(
            lines[2].starts_with("acme/oslo/packaging/speed,enterprise=acme;site=oslo;area=packaging,JSON,,5,false,"),
            "{}",
            lines[2]
        );
        assert!(
            lines[2].contains(",404,bpm,Good,value:number;unit:string;quality:string,,"),
            "{}",
            lines[2]
        );

        // JSON: what's known, with numbers as numbers and nothing empty.
        let j = crate::json::Json::parse(e.json.as_bytes(), 8).expect("valid JSON");
        assert_eq!(j.get("connection").and_then(|v| v.str()), Some("acme"));
        assert_eq!(j.get("root").and_then(|v| v.str()), Some("acme/oslo"));
        assert!(e.json.contains("\"messages\": 5"), "{}", e.json);
        assert!(
            e.json
                .contains("\"levels\": {\"enterprise\": \"acme\", \"site\": \"oslo\", \"area\": \"packaging\"}"),
            "{}",
            e.json
        );
        assert!(
            e.json
                .contains("\"fields\": {\"value\": \"number\", \"unit\": \"string\", \"quality\": \"string\"}"),
            "{}",
            e.json
        );
        assert!(e.json.contains("\"example\": \"hello, \\\"world\\\"\""), "{}", e.json);
        assert!(!e.json.contains("bergen"), "only below the root");
    }
}

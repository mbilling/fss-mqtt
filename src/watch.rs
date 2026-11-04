//! Watching topics for what looks wrong, as their messages arrive. Each
//! number a topic's messages carry (`chart::fields`) keeps a running mean
//! and spread: a value far outside them is a spike, and one that usually
//! changes but keeps repeating is stuck. The time between messages is kept
//! the same way: a topic that publishes regularly and goes quiet is silent.
//!
//! A JSON object payload's shape (its fields and their types) is learnt
//! too: a field changing type, one that was always there going, or a new
//! one, is drift. And a topic that doesn't fit its connection's unified
//! namespace (uns.rs) is flagged for as long as it's there.
//!
//! Each waits for enough messages to know what's usual, and the bounds are
//! wide, so a flag is worth a look rather than noise. It's cheap: a few
//! numbers per field, and constant time per message.

use web_time::{Duration, SystemTime};

use crate::json::Json;

/// Values a field (or gaps a topic) needs before it's judged.
const LEARN: u32 = 20;
/// The newest value's weight in the running mean and spread, once learnt:
/// about the last 30 count.
const ALPHA: f64 = 1.0 / 30.0;
/// A spike is this many standard deviations from the mean.
const SPIKE_SD: f64 = 6.0;
/// How long a spike stays flagged after the value.
pub const SPIKE_FOR: Duration = Duration::from_secs(300);
/// A field is stuck after repeating a value this many times, if it changed
/// in at least half its messages before.
const STUCK_REPEATS: u32 = 20;
/// A topic is silent after this many of its usual intervals without a
/// message (plus three times their spread), and at least `SILENT_MIN`.
const SILENT_GAPS: f64 = 4.0;
const SILENT_MIN: f64 = 5.0;
/// The most fields a topic's watch keeps, of numbers and of its shape.
const MAX_FIELDS: usize = 32;
const MAX_SHAPE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Spike,
    Stuck,
    Silent,
    /// The payload's shape changed.
    Drift,
    /// Not as the connection's unified namespace has it.
    Uns,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Spike => "spike",
            Kind::Stuck => "stuck",
            Kind::Silent => "silent",
            Kind::Drift => "drift",
            Kind::Uns => "uns",
        }
    }
}

/// A JSON object payload's fields, nested ones by their path ("meta.unit"),
/// each with its type; `None` for any other payload.
pub type Shape = Vec<(String, &'static str)>;

pub fn shape(payload: &[u8]) -> Option<Shape> {
    if payload.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
        return None;
    }
    let Ok(Json::Obj(fields)) = Json::parse(payload, 32) else {
        return None;
    };
    fn walk(fields: &[(String, Json)], path: &str, out: &mut Shape) {
        for (k, v) in fields {
            let name = if path.is_empty() {
                k.clone()
            } else {
                format!("{path}.{k}")
            };
            match v {
                Json::Obj(inner) => walk(inner, &name, out),
                _ if out.len() < MAX_SHAPE => out.push((name, type_of(v))),
                _ => return,
            }
        }
    }
    let mut out = Vec::new();
    walk(&fields, "", &mut out);
    Some(out)
}

fn type_of(j: &Json) -> &'static str {
    match j {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Num(_) => "number",
        Json::Str(_) => "string",
        Json::Arr(_) => "array",
        Json::Obj(_) => "object",
    }
}

/// A field of the shape learnt: its type, and in how many messages it was.
struct ShapeField {
    name: String,
    kind: &'static str,
    seen: u32,
}

/// What looks wrong, and since when.
#[derive(Clone, Debug, PartialEq)]
pub struct Flag {
    pub kind: Kind,
    /// "power_kw 9800 (usual 900–3600)"
    pub what: String,
    pub at: SystemTime,
}

/// A running mean and variance: exact while learning, then weighted to the
/// recent values.
#[derive(Default)]
struct Stats {
    n: u32,
    mean: f64,
    var: f64,
}

impl Stats {
    fn add(&mut self, x: f64) {
        let a = if self.n < LEARN {
            1.0 / (self.n as f64 + 1.0)
        } else {
            ALPHA
        };
        let d = x - self.mean;
        self.mean += a * d;
        self.var = (1.0 - a) * (self.var + a * d * d);
        self.n = self.n.saturating_add(1);
    }

    fn sd(&self) -> f64 {
        self.var.sqrt()
    }
}

struct Field {
    name: String,
    stats: Stats,
    last: f64,
    /// Times in a row `last` came again.
    repeats: u32,
    /// Times the value changed, ever.
    changes: u32,
}

/// A topic's watch.
#[derive(Default)]
pub struct Watch {
    fields: Vec<Field>,
    /// Seconds between messages.
    gaps: Stats,
    last: Option<SystemTime>,
    spike: Option<Flag>,
    stuck: Option<Flag>,
    silent: Option<Flag>,
    /// The JSON shape learnt, and how many JSON object payloads taught it.
    shape: Vec<ShapeField>,
    objects: u32,
    drift: Option<Flag>,
    uns: Option<Flag>,
}

impl Watch {
    /// A whole payload's shape (`None`: not a JSON object), as it came at
    /// `t`. Once learnt, a change is drift; then it's the shape learnt.
    pub fn shape(&mut self, t: SystemTime, now: Option<&Shape>) {
        let Some(now) = now else {
            if self.objects >= LEARN {
                self.drift = Some(Flag {
                    kind: Kind::Drift,
                    what: "no longer a JSON object".into(),
                    at: t,
                });
                self.objects = 0; // learn what it is now
                self.shape.clear();
            }
            return;
        };
        if self.objects >= LEARN {
            let mut changes = Vec::new();
            for (name, kind) in now {
                match self.shape.iter().find(|f| &f.name == name) {
                    // A null stands in for any value.
                    Some(f) if f.kind != *kind && *kind != "null" && f.kind != "null" => {
                        changes.push(format!("{name}: {} → {kind}", f.kind));
                    }
                    Some(_) => {}
                    None => changes.push(format!("new field {name}")),
                }
            }
            for f in &self.shape {
                if f.seen == self.objects && !now.iter().any(|(n, _)| n == &f.name) {
                    changes.push(format!("{} gone", f.name));
                }
            }
            if !changes.is_empty() {
                let more = changes.len().saturating_sub(3);
                changes.truncate(3);
                let mut what = changes.join(", ");
                if more > 0 {
                    what += &format!(" and {more} more");
                }
                self.drift = Some(Flag {
                    kind: Kind::Drift,
                    what,
                    at: t,
                });
            }
        }
        for (name, kind) in now {
            match self.shape.iter().position(|f| &f.name == name) {
                Some(i) => {
                    let f = &mut self.shape[i];
                    f.seen += 1;
                    if *kind != "null" {
                        f.kind = kind;
                    }
                }
                None if self.shape.len() < MAX_SHAPE => self.shape.push(ShapeField {
                    name: name.clone(),
                    kind,
                    seen: 1,
                }),
                None => {}
            }
        }
        self.objects += 1;
    }

    /// The JSON fields learnt, with their types.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.shape.iter().map(|f| (f.name.as_str(), f.kind))
    }

    /// Seconds between its messages, usually (once there were a few).
    pub fn every(&self) -> Option<f64> {
        (self.gaps.n >= 3).then_some(self.gaps.mean)
    }

    /// What's wrong with the topic in its connection's unified namespace,
    /// if anything (found at `t`).
    pub fn set_uns(&mut self, problem: Option<String>, t: SystemTime) {
        self.uns = match (problem, self.uns.take()) {
            (Some(what), Some(was)) if was.what == what => Some(was),
            (Some(what), _) => Some(Flag {
                kind: Kind::Uns,
                what,
                at: t,
            }),
            (None, _) => None,
        };
    }

    /// A message at `t` with these numbers.
    pub fn arrive(&mut self, t: SystemTime, numbers: &[(String, f64)]) {
        if let Some(prev) = self.last
            && let Ok(gap) = t.duration_since(prev)
        {
            self.gaps.add(gap.as_secs_f64());
        }
        self.last = Some(t);
        self.silent = None;
        let mut stuck = None;
        for (name, v) in numbers {
            let v = *v;
            let i = match self.fields.iter().position(|f| &f.name == name) {
                Some(i) => i,
                None if self.fields.len() < MAX_FIELDS => {
                    self.fields.push(Field {
                        name: name.clone(),
                        stats: Stats::default(),
                        last: v,
                        repeats: 0,
                        changes: 0,
                    });
                    self.fields.len() - 1
                }
                None => continue,
            };
            let f = &mut self.fields[i];
            let (n, mean, sd) = (f.stats.n, f.stats.mean, f.stats.sd());
            let mut x = v;
            if n >= LEARN && sd > 1e-9 * mean.abs().max(1.0) && (v - mean).abs() > SPIKE_SD * sd {
                let (lo, hi) = (mean - 3.0 * sd, mean + 3.0 * sd);
                self.spike = Some(Flag {
                    kind: Kind::Spike,
                    what: format!("{name} {} (usual {}–{})", num(v), num(lo), num(hi)),
                    at: t,
                });
                // A spike doesn't make the next one usual.
                x = v.clamp(mean - SPIKE_SD * sd, mean + SPIKE_SD * sd);
            }
            if n > 0 {
                if v == f.last {
                    f.repeats += 1;
                } else {
                    f.changes += 1;
                    f.repeats = 0;
                }
            }
            f.last = v;
            f.stats.add(x);
            // Moved in at least half its messages before this run of repeats.
            let before = f.stats.n.saturating_sub(1 + f.repeats);
            if f.repeats >= STUCK_REPEATS && before >= LEARN && f.changes * 2 >= before {
                stuck.get_or_insert(Flag {
                    kind: Kind::Stuck,
                    what: format!("{name} stuck at {} for {} messages", num(v), f.repeats + 1),
                    at: t,
                });
            }
        }
        // Stuck from when it started; the message count moves on.
        self.stuck = match (stuck, self.stuck.take()) {
            (Some(mut s), Some(was)) => {
                s.at = was.at;
                Some(s)
            }
            (s, _) => s,
        };
    }

    /// Whether the topic has gone quiet by `now`; true if that changed.
    pub fn check(&mut self, now: SystemTime) -> bool {
        let was = self.silent.is_some();
        let quiet = self
            .last
            .and_then(|l| now.duration_since(l).ok())
            .map(|d| d.as_secs_f64());
        let g = &self.gaps;
        // Only a topic that publishes regularly has a usual interval.
        let regular = g.n >= LEARN && g.sd() <= g.mean;
        let limit = (SILENT_GAPS * g.mean + 3.0 * g.sd()).max(SILENT_MIN);
        match quiet {
            Some(q) if regular && q > limit => {
                if self.silent.is_none() {
                    self.silent = Some(Flag {
                        kind: Kind::Silent,
                        what: format!("nothing since, usually every {}", secs(g.mean)),
                        at: self.last.unwrap_or(now),
                    });
                }
            }
            _ => self.silent = None,
        }
        was != self.silent.is_some()
    }

    /// What to flag at `now`: silent, else stuck, else a recent drift or
    /// spike, else what's wrong in the namespace.
    pub fn flag(&self, now: SystemTime) -> Option<&Flag> {
        let recent = |f: &&Flag| now.duration_since(f.at).map_or(true, |d| d < SPIKE_FOR);
        self.silent
            .as_ref()
            .or(self.stuck.as_ref())
            .or(self.drift.as_ref().filter(recent))
            .or(self.spike.as_ref().filter(recent))
            .or(self.uns.as_ref())
    }
}

/// A number as it reads well in a flag: up to 3 significant decimals.
fn num(x: f64) -> String {
    let s = if x.abs() >= 100.0 {
        format!("{x:.0}")
    } else if x.abs() >= 1.0 {
        format!("{x:.2}")
    } else {
        format!("{x:.3}")
    };
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

fn secs(s: f64) -> String {
    if s < 10.0 {
        format!("{} s", num(s))
    } else if s < 120.0 {
        format!("{s:.0} s")
    } else {
        format!("{:.0} min", s / 60.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use web_time::UNIX_EPOCH;

    fn at(s: f64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs_f64(1_000_000.0 + s)
    }

    /// Uniform noise in [lo, hi), from a fixed seed.
    fn noise(seed: &mut u64, lo: f64, hi: f64) -> f64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        lo + (*seed >> 11) as f64 / (1u64 << 53) as f64 * (hi - lo)
    }

    fn one(name: &str, v: f64) -> Vec<(String, f64)> {
        vec![(name.into(), v)]
    }

    #[test]
    fn noise_ramps_and_constants_raise_nothing() {
        let mut w = Watch::default();
        let mut seed = 0x9e37_79b9_7f4a_7c15;
        for i in 0..2000 {
            let t = at(i as f64);
            let nums = vec![
                ("power".to_string(), noise(&mut seed, 900.0, 3600.0)),
                ("hz".to_string(), noise(&mut seed, 49.95, 50.05)),
                ("counter".to_string(), i as f64),
                ("mode".to_string(), 3.0),
            ];
            w.arrive(t, &nums);
            assert!(w.flag(t).is_none(), "message {i}: {:?}", w.flag(t));
        }
    }

    #[test]
    fn a_spike_is_flagged_for_a_while() {
        let mut w = Watch::default();
        let mut seed = 7;
        for i in 0..100 {
            w.arrive(at(i as f64), &one("power_kw", noise(&mut seed, 900.0, 3600.0)));
        }
        // Too few values to know what's usual: nothing yet on a new field.
        w.arrive(at(100.0), &[("power_kw".into(), 9800.0), ("new".into(), 1e9)]);
        let f = w.flag(at(100.0)).expect("a spike");
        assert_eq!(f.kind, Kind::Spike);
        assert!(f.what.starts_with("power_kw 9800 (usual "), "{}", f.what);
        // Back to normal: still flagged, until SPIKE_FOR has passed.
        w.arrive(at(101.0), &one("power_kw", 2000.0));
        assert!(w.flag(at(101.0)).is_some());
        assert!(w.flag(at(100.0) + SPIKE_FOR + Duration::from_secs(1)).is_none());
    }

    #[test]
    fn a_value_that_stops_changing_is_stuck() {
        let mut w = Watch::default();
        let mut seed = 11;
        for i in 0..60 {
            w.arrive(at(i as f64), &one("wind_ms", noise(&mut seed, 4.0, 14.0)));
        }
        // The same value 20 times: 19 repeats, not yet.
        for i in 60..80 {
            w.arrive(at(i as f64), &one("wind_ms", 8.4));
            assert!(w.flag(at(i as f64)).is_none(), "not yet at {i}");
        }
        w.arrive(at(80.0), &one("wind_ms", 8.4));
        let f = w.flag(at(80.0)).expect("stuck");
        assert_eq!(
            (f.kind, f.what.as_str()),
            (Kind::Stuck, "wind_ms stuck at 8.4 for 21 messages")
        );
        w.arrive(at(81.0), &one("wind_ms", 8.4));
        let f = w.flag(at(81.0)).unwrap();
        assert_eq!(f.what, "wind_ms stuck at 8.4 for 22 messages");
        assert_eq!(f.at, at(80.0), "since it was first flagged");
        w.arrive(at(82.0), &one("wind_ms", 9.1));
        assert!(w.flag(at(82.0)).is_none(), "moving again");
    }

    #[test]
    fn a_payload_whose_shape_changes_drifts() {
        let obj = |p: &str| shape(p.as_bytes());
        assert_eq!(
            obj(r#"{"value": 21.5, "unit": "C", "meta": {"ok": true, "n": null}}"#).unwrap(),
            [
                ("value".to_string(), "number"),
                ("unit".to_string(), "string"),
                ("meta.ok".to_string(), "boolean"),
                ("meta.n".to_string(), "null")
            ]
        );
        assert_eq!(obj("21.5"), None);
        assert_eq!(obj("[1]"), None);

        let mut w = Watch::default();
        let ok = obj(r#"{"value": 21.5, "unit": "C", "note": null}"#);
        for i in 0..LEARN {
            w.shape(at(i as f64), ok.as_ref());
        }
        // A null where a number was isn't drift.
        w.shape(at(20.0), obj(r#"{"value": null, "unit": "C", "note": "x"}"#).as_ref());
        assert!(w.flag(at(20.0)).is_none());
        // The number as a string.
        w.shape(
            at(21.0),
            obj(r#"{"value": "21.5", "unit": "C", "note": null}"#).as_ref(),
        );
        let f = w.flag(at(21.0)).expect("drift");
        assert_eq!((f.kind, f.what.as_str()), (Kind::Drift, "value: number → string"));
        // Learnt: the same again isn't, until it changes once more.
        w.shape(
            at(22.0),
            obj(r#"{"value": "21.5", "unit": "C", "note": null}"#).as_ref(),
        );
        assert_eq!(w.flag(at(22.0)).unwrap().at, at(21.0));
        w.shape(at(23.0), obj(r#"{"value": "21.5", "note": null, "q": 1}"#).as_ref());
        assert_eq!(w.flag(at(23.0)).unwrap().what, "new field q, unit gone");
        assert!(w.flag(at(23.0) + SPIKE_FOR + Duration::from_secs(1)).is_none());
        w.shape(at(24.0), None);
        assert_eq!(w.flag(at(24.0)).unwrap().what, "no longer a JSON object");
    }

    #[test]
    fn a_namespace_problem_stays_flagged_until_its_gone() {
        let mut w = Watch::default();
        w.set_uns(Some("missing the cell level".into()), at(0.0));
        assert_eq!(w.flag(at(9999.0)).unwrap().kind, Kind::Uns);
        w.set_uns(Some("missing the cell level".into()), at(5.0));
        assert_eq!(w.flag(at(5.0)).unwrap().at, at(0.0), "since it was first found");
        w.set_uns(None, at(6.0));
        assert!(w.flag(at(6.0)).is_none());
    }

    #[test]
    fn a_regular_topic_that_goes_quiet_is_silent() {
        let mut w = Watch::default();
        for i in 0..30 {
            w.arrive(at(i as f64), &[]);
        }
        assert!(!w.check(at(33.0)), "5 s at least");
        assert!(w.check(at(36.0)));
        let f = w.flag(at(36.0)).unwrap();
        assert_eq!(
            (f.kind, f.what.as_str()),
            (Kind::Silent, "nothing since, usually every 1 s")
        );
        assert_eq!(f.at, at(29.0));
        w.arrive(at(40.0), &[]);
        assert!(w.flag(at(40.0)).is_none());

        // Irregular: no usual interval, never silent.
        let mut w = Watch::default();
        let mut t = 0.0;
        let mut seed = 3;
        for _ in 0..50 {
            t += noise(&mut seed, 0.0, 1.0).powi(4) * 600.0;
            w.arrive(at(t), &[]);
        }
        assert!(!w.check(at(t + 3600.0)));
    }
}

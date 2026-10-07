//! Rendering. Everything is drawn straight into the frame buffer.

use std::time::Duration;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::app::{App, Focus, msg_index};
use crate::fmt::{self, ago, commas, human_bytes, human_count, msg_count, plural};
use crate::store::{ConnState, FilterMode, MATCHED, Message, Row, Store};
use crate::theme::Theme;

const ACTIVE_FOR: Duration = Duration::from_millis(1500);

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < 10 || area.height < 8 {
        return;
    }
    let st = app.store.lock().unwrap();
    let buf = f.buffer_mut();

    if app.viewer.is_some() {
        viewer(buf, area, app);
        return;
    }

    let l = app.layout();
    put(buf, 0, 0, header(app, &st), area.width);
    let tree_area = Rect::new(0, 1, l.tree_w, l.tree_h);
    let det_area = if l.stacked {
        Rect::new(0, 1 + l.tree_h, l.det_w, l.det_h)
    } else {
        Rect::new(l.tree_w, 1, l.det_w, l.det_h)
    };
    tree_box(buf, tree_area, app, &st);
    detail_box(buf, det_area, app, &st);
    let y = det_area.bottom();
    input_box(buf, Rect::new(0, y, area.width, 3), app);
    put(buf, 0, y + 3, hints(app), area.width);
}

fn put(buf: &mut Buffer, x: u16, y: u16, line: Line, w: u16) {
    if y < buf.area.bottom() {
        let line = fmt::fit(line.spans, w as usize);
        buf.set_line(x, y, &line, w);
    }
}

/// Rounded box with a title in the top edge.
fn draw_box(buf: &mut Buffer, r: Rect, title: &str, lines: Vec<Line>, focused: bool, th: &Theme) {
    if r.width < 4 || r.height < 2 {
        return;
    }
    let (bs, ts) = if focused {
        (th.dim, th.accent_b)
    } else {
        (th.border, th.dim)
    };
    let inner = (r.width - 4) as usize;
    let mut top = vec![Span::styled("╭─", bs)];
    let mut used = 2;
    if !title.is_empty() {
        let t = format!(" {} ", fmt::trunc_left(title, inner.saturating_sub(2)));
        used += fmt::str_width(&t);
        top.push(Span::styled(t, ts));
    }
    let fill = (r.width as usize).saturating_sub(used + 1);
    top.push(Span::styled("─".repeat(fill) + "╮", bs));
    buf.set_line(r.x, r.y, &Line::from(top), r.width);

    let mut lines = lines.into_iter();
    for y in r.y + 1..r.bottom() - 1 {
        buf.set_string(r.x, y, "│", bs);
        buf.set_string(r.right() - 1, y, "│", bs);
        if let Some(l) = lines.next() {
            put(buf, r.x + 2, y, l, inner as u16);
        }
    }
    buf.set_string(
        r.x,
        r.bottom() - 1,
        format!("╰{}╯", "─".repeat(r.width as usize - 2)),
        bs,
    );
}

fn header<'a>(app: &App, st: &Store) -> Line<'a> {
    let th = &app.th;
    let (dot, state) = match st.status.state {
        ConnState::Connected => (Span::styled("●", th.green), "connected"),
        ConnState::Disconnected => (Span::styled("●", th.red), "disconnected"),
        ConnState::Connecting => (Span::styled("◌", th.yellow), "connecting"),
    };
    let mut state = state.to_string();
    if let Some(e) = &st.status.err {
        state += &format!(" · {e}");
    }
    let left = vec![
        Span::raw(" "),
        Span::styled("✻ fss-mqtt", th.accent_b),
        Span::raw("  "),
        Span::styled(app.opts.broker.clone(), th.dim),
        Span::raw("  "),
        dot,
        Span::raw(" "),
        Span::styled(state, th.dim),
    ];
    let right = vec![
        Span::styled(commas(st.topic_count() as u64), th.text),
        Span::styled(" topics  ", th.dim),
        Span::styled(human_count(st.total()), th.text),
        Span::styled(" msgs  ", th.dim),
        Span::styled(format!("{:.0}", app.rate), th.text),
        Span::styled("/s ", th.dim),
    ];
    join_lr(left, right, app.width as usize)
}

/// Left spans, then right spans flush right; right is dropped if it doesn't fit.
fn join_lr<'a>(mut left: Vec<Span<'a>>, right: Vec<Span<'a>>, w: usize) -> Line<'a> {
    let lw: usize = left.iter().map(|s| s.width()).sum();
    let rw: usize = right.iter().map(|s| s.width()).sum();
    if lw + rw + 1 > w {
        return Line::from(left);
    }
    left.push(Span::raw(" ".repeat(w - lw - rw)));
    left.extend(right);
    Line::from(left)
}

fn tree_box(buf: &mut Buffer, r: Rect, app: &App, st: &Store) {
    let th = &app.th;
    let (ih, iw) = (r.height.saturating_sub(2) as usize, r.width.saturating_sub(4) as usize);
    let mut lines = Vec::new();
    if app.rows.is_empty() {
        lines.push(Line::default());
        let refused = st.status.err.as_deref().filter(|e| e.starts_with("subscribe"));
        if st.topic_count() == 0 {
            if let Some(e) = refused {
                lines.push(Line::styled(e.to_string(), th.red));
                lines.push(Line::default());
                lines.push(Line::styled(
                    "The broker refused the subscription. Pick topics this user may read, e.g.",
                    th.dim,
                ));
                lines.push(Line::styled("  fss-mqtt … -t 'devices/+/up/#'", th.text));
            } else {
                lines.push(Line::styled(
                    format!("Waiting for messages on {} …", app.opts.topics.join(", ")),
                    th.dim,
                ));
            }
        } else {
            lines.push(Line::from(vec![
                Span::styled("Nothing matches ", th.dim),
                Span::styled(app.filter_text.clone(), th.accent),
            ]));
        }
    }
    let end = (app.offset + ih).min(app.rows.len());
    for i in app.offset..end {
        lines.push(tree_line(app, st, &app.rows[i], i == app.cursor, iw));
    }
    let title = if app.rows.len() > ih {
        format!("Topics {}/{}", app.cursor + 1, app.rows.len())
    } else {
        "Topics".into()
    };
    draw_box(buf, r, &title, lines, app.focus == Focus::Tree, th);
}

fn tree_line<'a>(app: &App, st: &Store, r: &Row, sel: bool, w: usize) -> Line<'a> {
    let th = &app.th;
    let n = &st.nodes[r.node];
    let mut spans = vec![match (sel, app.focus) {
        (true, Focus::Tree) => Span::styled("❯ ", th.accent_b),
        (true, _) => Span::styled("› ", th.dim),
        _ => Span::raw("  "),
    }];
    spans.push(Span::raw("  ".repeat(n.depth - 1)));

    let active = app
        .now
        .duration_since(n.last_seen)
        .map(|d| d < ACTIVE_FOR)
        .unwrap_or(true);
    let gs = if active { th.green } else { th.faint };
    spans.push(Span::styled(
        if r.expanded {
            "▾ "
        } else if n.has_children() {
            "▸ "
        } else {
            "• "
        },
        gs,
    ));
    spans.extend(node_name(app, &n.name, r.mark, sel));

    if let Some(last) = n.latest() {
        if last.size <= app.opts.inline_bytes {
            spans.push(Span::styled(" = ", th.faint));
            spans.push(Span::styled(
                fmt::one_line(&last.payload, app.opts.inline_bytes),
                th.value,
            ));
        } else {
            spans.push(Span::styled(format!(" [{}]", human_bytes(last.size)), th.faint));
        }
    }
    if n.has_children() && !r.expanded {
        spans.push(Span::styled(format!(" ({})", n.topics), th.faint));
    }
    join_lr(spans, vec![Span::styled(human_count(n.sub_msgs), th.faint)], w)
}

fn node_name<'a>(app: &App, name: &str, mark: u8, sel: bool) -> Vec<Span<'a>> {
    let th = &app.th;
    let base = if sel { th.bold } else { th.text };
    if name.is_empty() {
        return vec![Span::styled("∅", th.dim.add_modifier(Modifier::ITALIC))];
    }
    if mark & MATCHED == 0 {
        return vec![Span::styled(name.to_string(), base)];
    }
    if app.filter.mode == FilterMode::Pattern {
        return vec![Span::styled(name.to_string(), th.matched)];
    }
    // Search: underline each occurrence of the query.
    let q = app.filter_text.to_lowercase();
    let lower = name.to_lowercase();
    if lower.len() != name.len() || q.is_empty() {
        return vec![Span::styled(name.to_string(), th.matched)];
    }
    let hl = th.matched.add_modifier(Modifier::UNDERLINED);
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(j) = lower[i..].find(&q) {
        if j > 0 {
            out.push(Span::styled(name[i..i + j].to_string(), base));
        }
        out.push(Span::styled(name[i + j..i + j + q.len()].to_string(), hl));
        i += j + q.len();
    }
    if i < name.len() {
        out.push(Span::styled(name[i..].to_string(), base));
    }
    out
}

fn detail_box(buf: &mut Buffer, r: Rect, app: &App, st: &Store) {
    let Some(n) = app.selected else {
        draw_box(buf, r, "", vec![], false, &app.th);
        return;
    };
    let node = &st.nodes[n];
    let (iw, ih) = (r.width.saturating_sub(4) as usize, r.height.saturating_sub(2) as usize);
    let mut lines = if node.has_messages() {
        topic_detail(app, st, n, iw)
    } else {
        branch_detail(app, st, n, iw)
    };
    lines.truncate(ih);
    draw_box(buf, r, &node.path, lines, app.focus == Focus::Msgs, &app.th);
}

fn topic_detail<'a>(app: &App, st: &Store, n: usize, w: usize) -> Vec<Line<'a>> {
    let th = &app.th;
    let node = &st.nodes[n];
    let msgs = node.messages();
    let idx = if app.focus == Focus::Msgs {
        msg_index(&msgs, app.msg_sel)
    } else {
        0
    };
    let cur = &msgs[idx];
    let mut out: Vec<Line> = Vec::new();

    let mut meta = vec![Span::styled(
        format!(
            "{} · last {} · qos {}",
            msg_count(node.msg_count),
            ago(app.now, cur.time),
            cur.qos
        ),
        th.dim,
    )];
    if cur.retain {
        meta.push(Span::styled(" · ", th.dim));
        meta.push(Span::styled("retained", th.yellow));
    }
    if node.has_children() {
        meta.push(Span::styled(
            format!(" · {}", plural(node.num_children(), "child", "children")),
            th.dim,
        ));
    }
    out.push(Line::from(meta));
    out.push(Line::default());

    // Messages
    let mut hdr = vec![Span::styled("Messages", th.section)];
    if app.focus == Focus::Msgs {
        hdr.push(Span::styled(format!("  {}/{}", idx + 1, msgs.len()), th.dim));
    }
    out.push(Line::from(hdr));
    for (i, m) in msgs.iter().enumerate() {
        let (mark, ts) = match (i == idx, app.focus) {
            (true, Focus::Msgs) => (Span::styled("❯ ", th.accent_b), th.text),
            (true, _) => (Span::styled("› ", th.faint), th.dim),
            _ => (Span::raw("  "), th.dim),
        };
        let mut spans = vec![
            mark,
            Span::styled(fmt::clock(m.time, false), ts),
            Span::raw(" "),
            if m.retain {
                Span::styled("R", th.yellow)
            } else {
                Span::raw(" ")
            },
            Span::raw(" "),
            Span::styled(format!("{:>8}", human_bytes(m.size)), th.dim),
            Span::raw("  "),
        ];
        let used: usize = spans.iter().map(|s| s.width()).sum();
        if w > used {
            spans.push(Span::styled(fmt::one_line(&m.payload, (w - used) * 4), th.value));
        }
        out.push(Line::from(spans));
    }
    out.push(Line::default());

    // Properties
    let props = message_props(cur);
    if !props.is_empty() {
        out.push(Line::styled("Properties", th.section));
        let kw = props.iter().map(|p| p.0.len()).max().unwrap_or(0).min(w / 2);
        for (k, v) in props {
            out.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(fmt::pad(&k, kw), th.purple),
                Span::raw("  "),
                Span::styled(fmt::one_line(v.as_bytes(), 512), th.text),
            ]));
        }
        out.push(Line::default());
    }

    // Payload preview
    let shown = &cur.payload[..cur.payload.len().min(app.opts.preview_bytes)];
    let mut ph = vec![
        Span::styled("Payload ", th.section),
        Span::styled(format!("· {}", human_bytes(cur.size)), th.dim),
    ];
    if shown.len() < cur.size {
        ph.push(Span::styled(format!(" · first {} B", shown.len()), th.dim));
    }
    out.push(Line::from(ph));
    if cur.size == 0 {
        out.push(Line::styled("  (empty)", th.dim.add_modifier(Modifier::ITALIC)));
    } else if fmt::is_text(shown) {
        for l in fmt::wrap(&fmt::one_line(shown, shown.len()), w.saturating_sub(2)) {
            out.push(Line::from(vec![Span::raw("  "), Span::styled(l, th.value)]));
        }
    } else {
        for mut l in fmt::hex_lines(shown, w.saturating_sub(2), th) {
            l.spans.insert(0, Span::raw("  "));
            out.push(l);
        }
    }
    out.push(Line::default());
    out.push(Line::styled("  ⏎ to view the full message", th.faint));
    out
}

/// The MQTT v5 properties worth showing, user properties last.
pub fn message_props(m: &Message) -> Vec<(String, String)> {
    let p = &m.props;
    let mut out = Vec::new();
    if let Some(v) = &p.content_type {
        out.push(("content-type".into(), v.clone()));
    }
    if let Some(v) = p.payload_format {
        out.push(("payload-format".into(), if v == 1 { "utf-8" } else { "bytes" }.into()));
    }
    if let Some(v) = &p.response_topic {
        out.push(("response-topic".into(), v.clone()));
    }
    if let Some(v) = &p.correlation_data {
        let s = if fmt::is_text(v) {
            String::from_utf8_lossy(v).into_owned()
        } else {
            v.iter().map(|b| format!("{b:02x}")).collect()
        };
        out.push(("correlation-data".into(), s));
    }
    if let Some(v) = p.message_expiry {
        out.push(("message-expiry".into(), format!("{v}s")));
    }
    if let Some(v) = p.subscription_id {
        out.push(("subscription-id".into(), v.to_string()));
    }
    out.extend(p.user.iter().cloned());
    out
}

fn branch_detail<'a>(app: &App, st: &Store, n: usize, w: usize) -> Vec<Line<'a>> {
    let th = &app.th;
    let node = &st.nodes[n];
    let mut out = vec![
        Line::styled(
            format!(
                "{} · {} · {} · last {}",
                plural(node.topics as usize, "topic", "topics"),
                msg_count(node.sub_msgs),
                plural(node.num_children(), "child", "children"),
                ago(app.now, node.last_seen)
            ),
            th.dim,
        ),
        Line::default(),
        Line::styled("Recently active", th.section),
    ];
    let mut kids: Vec<usize> = node.children().to_vec();
    kids.sort_by(|&a, &b| st.nodes[b].last_seen.cmp(&st.nodes[a].last_seen));
    kids.truncate(12);
    let nw = kids
        .iter()
        .map(|&k| fmt::str_width(&st.nodes[k].name))
        .max()
        .unwrap_or(0)
        .min(w / 2);
    for k in kids {
        let kn = &st.nodes[k];
        let active = app
            .now
            .duration_since(kn.last_seen)
            .map(|d| d < ACTIVE_FOR)
            .unwrap_or(true);
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("• ", if active { th.green } else { th.faint }),
            Span::styled(fmt::pad(&kn.name, nw), th.text),
            Span::raw("  "),
            Span::styled(
                format!("{:<8} {}", ago(app.now, kn.last_seen), msg_count(kn.sub_msgs)),
                th.dim,
            ),
        ]));
    }
    out
}

fn input_box(buf: &mut Buffer, r: Rect, app: &App) {
    let th = &app.th;
    let inner = r.width.saturating_sub(4) as usize;
    let info = match app.filter.mode {
        FilterMode::Pattern => vec![
            Span::styled("pattern · ", th.dim),
            Span::styled(plural(app.matched as usize, "topic", "topics"), th.text),
        ],
        FilterMode::Search => vec![
            Span::styled("search · ", th.dim),
            Span::styled(plural(app.matched as usize, "topic", "topics"), th.text),
        ],
        FilterMode::None => vec![],
    };
    let mut left = vec![Span::styled("> ", th.accent_b)];
    if app.filter_text.is_empty() {
        left.push(Span::styled("T", th.cursor));
        left.push(Span::styled(
            "ype to filter  ·  v1/#   sensor/+/temp   or any text",
            th.faint,
        ));
    } else {
        left.push(Span::styled(app.filter_text.clone(), th.text));
        left.push(Span::styled(" ", th.cursor));
    }
    let line = join_lr(left, info, inner);
    let bs = th.border;
    let bar = "─".repeat(r.width.saturating_sub(2) as usize);
    buf.set_string(r.x, r.y, format!("╭{bar}╮"), bs);
    buf.set_string(r.x, r.y + 1, "│", bs);
    put(buf, r.x + 2, r.y + 1, line, inner as u16);
    buf.set_string(r.right() - 1, r.y + 1, "│", bs);
    buf.set_string(r.x, r.y + 2, format!("╰{bar}╯"), bs);
}

fn hint<'a>(th: &Theme, pairs: &[(&str, &str)]) -> Line<'a> {
    let mut spans = vec![Span::raw("  ")];
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ·  ", th.faint));
        }
        spans.push(Span::styled(k.to_string(), th.text));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(v.to_string(), th.dim));
    }
    Line::from(spans)
}

fn hints<'a>(app: &App) -> Line<'a> {
    if app.focus == Focus::Msgs {
        hint(
            &app.th,
            &[
                ("↑↓", "messages"),
                ("⏎", "open"),
                ("← esc", "back to tree"),
                ("type", "filter"),
                ("^c", "quit"),
            ],
        )
    } else {
        hint(
            &app.th,
            &[
                ("↑↓", "move"),
                ("←→", "collapse/expand"),
                ("⏎", "open"),
                ("esc", "clear filter"),
                ("^w", "up a level"),
                ("^c", "quit"),
            ],
        )
    }
}

fn viewer(buf: &mut Buffer, area: Rect, app: &App) {
    let v = app.viewer.as_ref().unwrap();
    let th = &app.th;
    let body_h = app.viewer_body_h();
    let mut lines = v.head.clone();
    let end = (v.scroll + body_h).min(v.lines.len());
    lines.extend(v.lines[v.scroll.min(end)..end].iter().cloned());
    draw_box(
        buf,
        Rect::new(0, 0, area.width, area.height - 1),
        &v.path,
        lines,
        true,
        th,
    );

    let mut h = hint(
        th,
        &[
            ("↑↓ pgup pgdn", "scroll"),
            ("←→", "older/newer"),
            ("tab", &format!("view: {}", v.mode_name())),
            ("esc", "back"),
        ],
    );
    if v.lines.len() > body_h {
        h.spans.push(Span::styled(
            format!("  {}–{} of {} lines", v.scroll + 1, end, v.lines.len()),
            th.faint,
        ));
    }
    put(buf, 0, area.height - 1, h, area.width);
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::app::{App, Focus, Options};
    use crate::store::{Props, Store};
    use crate::theme::Theme;

    fn sample() -> App {
        let mut st = Store::new(10, 64 * 1024);
        for i in 0..3 {
            for line in ["line1", "line2", "line10"] {
                let props = Props {
                    content_type: Some("application/json".into()),
                    user: vec![("site".into(), "oslo".into()), ("unit".into(), "C".into())],
                    ..Default::default()
                };
                let p = format!(r#"{{"t":{}.5,"ok":true,"tags":["a","b"],"note":null}}"#, 20 + i);
                st.add(&format!("v1/plant/{line}/temp"), p.as_bytes(), 1, false, props);
                st.add(
                    &format!("v1/plant/{line}/status"),
                    b"RUNNING",
                    0,
                    false,
                    Props::default(),
                );
            }
        }
        st.add("v1/camera/frame", &vec![0u8; 200_000], 0, false, Props::default());
        st.add("v2/fleet/truck7/gps", b"59.91,10.75", 0, true, Props::default());
        st.add(
            "v2/fleet/truck7/bin",
            &[0, 1, 2, 0xff, b'h', b'i'],
            0,
            false,
            Props::default(),
        );
        let opts = Options {
            broker: "mqtt://localhost:1883".into(),
            topics: vec!["#".into()],
            preview_bytes: 50,
            inline_bytes: 64,
        };
        let mut app = App::new(Arc::new(Mutex::new(st)), opts, Theme::new(false));
        app.resize(110, 30);
        app.tick();
        app
    }

    fn typ(app: &mut App, s: &str) {
        for c in s.chars() {
            app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }
    fn press(app: &mut App, codes: &[KeyCode]) {
        for &c in codes {
            app.key(KeyEvent::new(c, KeyModifiers::NONE));
        }
    }

    fn frame(app: &App, label: &str) -> String {
        let mut t = Terminal::new(TestBackend::new(app.width, app.height)).unwrap();
        t.draw(|f| super::draw(f, app)).unwrap();
        let buf = t.backend().buffer();
        let mut s = String::new();
        for y in 0..buf.area.height {
            let mut line = String::new();
            for x in 0..buf.area.width {
                line.push_str(buf[(x, y)].symbol());
            }
            s.push_str(line.trim_end());
            s.push('\n');
        }
        println!("── {label} ──\n{s}");
        s
    }

    #[test]
    fn filter_and_navigate() {
        let mut app = sample();
        let v = frame(&app, "start");
        assert!(v.contains("v1") && v.contains("v2"));

        typ(&mut app, "v1/+/line1");
        let v = frame(&app, "filter v1/+/line1");
        assert!(v.contains("line1") && !v.contains("line2") && !v.contains("truck7"));

        press(&mut app, &[KeyCode::Down, KeyCode::Down]);
        let path = |a: &App| a.store.lock().unwrap().nodes[a.selected.unwrap()].path.clone();
        assert_eq!(path(&app), "v1/plant/line1/temp");
        let v = frame(&app, "on leaf");
        for want in ["site", "oslo", "content-type", r#"{"t":22.5"#] {
            assert!(v.contains(want), "detail pane missing {want}");
        }

        press(&mut app, &[KeyCode::Right, KeyCode::Down, KeyCode::Down]);
        assert!(app.focus == Focus::Msgs);
        frame(&app, "message list");

        press(&mut app, &[KeyCode::Enter]);
        assert!(app.viewer.is_some());
        let v = frame(&app, "viewer");
        assert!(
            v.contains(r#""t": 20.5"#),
            "viewer should pretty-print the oldest message"
        );
        press(&mut app, &[KeyCode::Tab, KeyCode::Tab]);
        frame(&app, "viewer hex");
        press(&mut app, &[KeyCode::Esc, KeyCode::Left]);
        assert!(app.focus == Focus::Tree && app.viewer.is_none());

        press(&mut app, &[KeyCode::Esc]);
        frame(&app, "filter cleared, path kept open");
        assert_eq!(path(&app), "v1/plant/line1/temp");

        typ(&mut app, "truck");
        press(&mut app, &[KeyCode::Down, KeyCode::Down]);
        frame(&app, "search truck");
        app.resize(60, 30);
        frame(&app, "narrow");
    }
}

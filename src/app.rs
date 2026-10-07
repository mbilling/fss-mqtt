//! UI state and key handling.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::store::{Filter, FilterMode, INSIDE, MATCHED, Message, ROOT, Row, Store};
use crate::theme::Theme;
use crate::viewer::Viewer;

/// Up to this many matching topics, a filter expands everything it matched.
const AUTO_EXPAND_LIMIT: u32 = 300;

pub struct Options {
    pub broker: String,
    pub topics: Vec<String>,
    pub preview_bytes: usize, // payload bytes shown in the detail pane
    pub inline_bytes: usize,  // payloads up to this size are shown inline in the tree
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Msgs,
}

pub struct Layout {
    pub tree_w: u16,
    pub tree_h: u16,
    pub det_w: u16,
    pub det_h: u16,
    pub stacked: bool,
}

pub struct App {
    pub store: Arc<Mutex<Store>>,
    pub opts: Options,
    pub th: Theme,
    pub width: u16,
    pub height: u16,
    pub now: SystemTime,

    pub filter_text: String,
    pub filter: Filter,
    pub marks: Option<Vec<u8>>,
    pub matched: u32,

    pub rows: Vec<Row>,
    pub cursor: usize,
    pub offset: usize,
    pub selected: Option<usize>,

    expanded: HashSet<usize>,   // used when no filter is active
    fexp: HashMap<usize, bool>, // overrides while a filter is active
    struct_ver: u64,
    need_rebuild: bool,

    pub focus: Focus,
    pub msg_sel: Option<u64>, // seq of the selected message in the list

    pub viewer: Option<Viewer>,

    pub rate: f64,
    rate_total: u64,
    rate_at: Instant,
    pub quit: bool,
}

impl App {
    pub fn new(store: Arc<Mutex<Store>>, opts: Options, th: Theme) -> App {
        App {
            store,
            opts,
            th,
            width: 0,
            height: 0,
            now: SystemTime::now(),
            filter_text: String::new(),
            filter: Filter::compile(""),
            marks: None,
            matched: 0,
            rows: Vec::new(),
            cursor: 0,
            offset: 0,
            selected: None,
            expanded: HashSet::new(),
            fexp: HashMap::new(),
            struct_ver: u64::MAX,
            need_rebuild: true,
            focus: Focus::Tree,
            msg_sel: None,
            viewer: None,
            rate: 0.0,
            rate_total: 0,
            rate_at: Instant::now(),
            quit: false,
        }
    }

    pub fn layout(&self) -> Layout {
        let body_h = self.height.saturating_sub(5).max(4); // header + input box (3) + hints
        if self.width < 80 {
            let th = body_h * 55 / 100;
            return Layout {
                tree_w: self.width,
                tree_h: th,
                det_w: self.width,
                det_h: body_h - th,
                stacked: true,
            };
        }
        let tw = (self.width as u32 * 45 / 100) as u16;
        Layout {
            tree_w: tw,
            tree_h: body_h,
            det_w: self.width - tw,
            det_h: body_h,
            stacked: false,
        }
    }

    pub fn tree_height(&self) -> usize {
        self.layout().tree_h.saturating_sub(2) as usize
    }

    pub fn resize(&mut self, w: u16, h: u16) {
        self.width = w;
        self.height = h;
        if let Some(v) = &mut self.viewer {
            v.relayout(w.saturating_sub(4) as usize, &self.th);
        }
        self.clamp_cursor();
    }

    pub fn tick(&mut self) {
        self.now = SystemTime::now();
        let total = {
            let st = self.store.lock().unwrap();
            if st.struct_ver != self.struct_ver {
                self.need_rebuild = true;
            }
            st.total()
        };
        let d = self.rate_at.elapsed();
        if d >= Duration::from_secs(1) {
            self.rate = (total - self.rate_total) as f64 / d.as_secs_f64();
            self.rate_total = total;
            self.rate_at = Instant::now();
        }
        self.rebuild(false);
    }

    /// Recomputes visible rows if the tree, filter or expansion changed.
    fn rebuild(&mut self, filter_changed: bool) {
        if !self.need_rebuild && !filter_changed {
            return;
        }
        self.need_rebuild = false;
        let store = self.store.clone();
        let mut st = store.lock().unwrap();
        st.sort_children();
        self.struct_ver = st.struct_ver;
        let (marks, matched) = self.filter.visibility(&st);
        self.marks = marks;
        self.matched = matched;
        self.rows = crate::store::flatten(&st, self.marks.as_deref(), &|n, m| self.is_expanded(n, m));
        drop(st);

        if filter_changed {
            // Like fzf: jump to the first thing that matched.
            self.cursor = self.rows.iter().position(|r| r.mark & MATCHED != 0).unwrap_or(0);
            self.offset = 0;
        } else if let Some(sel) = self.selected
            && let Some(i) = self.rows.iter().position(|r| r.node == sel)
        {
            self.cursor = i;
        }
        self.clamp_cursor();
    }

    fn is_expanded(&self, n: usize, mark: u8) -> bool {
        if self.filter.mode == FilterMode::None {
            return self.expanded.contains(&n);
        }
        if let Some(&v) = self.fexp.get(&n) {
            return v;
        }
        mark & INSIDE == 0 || self.matched <= AUTO_EXPAND_LIMIT
    }

    fn set_expanded(&mut self, n: usize, v: bool) {
        if self.filter.mode == FilterMode::None {
            if v {
                self.expanded.insert(n);
            } else {
                self.expanded.remove(&n);
            }
        } else {
            self.fexp.insert(n, v);
        }
        self.need_rebuild = true;
    }

    fn move_cursor(&mut self, delta: isize) {
        self.cursor = (self.cursor as isize + delta).max(0) as usize;
    }

    fn clamp_cursor(&mut self) {
        if self.cursor >= self.rows.len() {
            self.cursor = self.rows.len().saturating_sub(1);
        }
        let prev = self.selected;
        self.selected = self.rows.get(self.cursor).map(|r| r.node);
        if self.selected != prev {
            self.msg_sel = None;
            self.focus = Focus::Tree;
        }
        let h = self.tree_height();
        if self.cursor < self.offset {
            self.offset = self.cursor;
        }
        if h > 0 && self.cursor >= self.offset + h {
            self.offset = self.cursor + 1 - h;
        }
        self.offset = self.offset.min(self.rows.len().saturating_sub(h));
    }

    fn set_filter(&mut self, s: String) {
        if s == self.filter_text {
            return;
        }
        let was_filtered = self.filter.mode != FilterMode::None;
        self.filter = Filter::compile(&s);
        self.filter_text = s;
        self.fexp.clear();
        self.focus = Focus::Tree;
        if self.filter.mode == FilterMode::None {
            // Clearing the filter: open the path to where you were.
            if let (true, Some(sel)) = (was_filtered, self.selected) {
                let st = self.store.lock().unwrap();
                let mut p = st.nodes[sel].parent;
                while let Some(i) = p {
                    if i == ROOT {
                        break;
                    }
                    self.expanded.insert(i);
                    p = st.nodes[i].parent;
                }
            }
            self.need_rebuild = true;
            self.rebuild(false);
            return;
        }
        self.rebuild(true);
    }

    pub fn paste(&mut self, s: &str) {
        if self.viewer.is_none() {
            let t = format!("{}{}", self.filter_text, s.trim());
            self.set_filter(t);
        }
    }

    pub fn key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.viewer.is_some() {
            self.viewer_key(k);
            return;
        }
        match k.code {
            KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => {
                let t = format!("{}{c}", self.filter_text);
                self.set_filter(t);
                return;
            }
            KeyCode::Backspace => {
                let mut t = self.filter_text.clone();
                t.pop();
                self.set_filter(t);
                return;
            }
            KeyCode::Char('w') if ctrl => {
                // delete back to the previous level
                let s = self.filter_text.trim_end_matches('/');
                let t = match s.rfind('/') {
                    Some(i) => s[..=i].to_string(),
                    None => String::new(),
                };
                self.set_filter(t);
                return;
            }
            KeyCode::Char('u') if ctrl => {
                self.set_filter(String::new());
                return;
            }
            _ => {}
        }

        if self.focus == Focus::Msgs {
            self.msg_key(k);
            return;
        }

        let Some(n) = self.selected else {
            if k.code == KeyCode::Esc {
                self.set_filter(String::new());
            }
            return;
        };
        let (has_kids, has_msgs, parent) = {
            let st = self.store.lock().unwrap();
            let node = &st.nodes[n];
            (node.has_children(), node.has_messages(), node.parent)
        };
        let expanded = self.rows[self.cursor].expanded;
        let page = self.tree_height() as isize;
        match k.code {
            KeyCode::Esc => self.set_filter(String::new()),
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-page),
            KeyCode::PageDown => self.move_cursor(page),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.rows.len().saturating_sub(1),
            KeyCode::Right => {
                if has_kids && !expanded {
                    self.set_expanded(n, true);
                } else if has_msgs {
                    self.enter_msgs();
                } else if expanded {
                    self.move_cursor(1);
                }
            }
            KeyCode::Left => {
                if expanded {
                    self.set_expanded(n, false);
                } else if let Some(p) = parent.filter(|&p| p != ROOT)
                    && let Some(i) = self.rows[..self.cursor].iter().rposition(|r| r.node == p)
                {
                    self.cursor = i;
                }
            }
            KeyCode::Tab if has_msgs => self.enter_msgs(),
            KeyCode::Enter => {
                if has_msgs {
                    self.open_viewer(n, None);
                } else if has_kids {
                    self.set_expanded(n, !expanded);
                }
            }
            _ => {}
        }
        self.clamp_cursor();
        self.rebuild(false);
    }

    fn enter_msgs(&mut self) {
        if let Some(n) = self.selected {
            let st = self.store.lock().unwrap();
            self.msg_sel = st.nodes[n].latest().map(|m| m.seq);
            drop(st);
            self.focus = Focus::Msgs;
        }
    }

    fn msg_key(&mut self, k: KeyEvent) {
        let Some(n) = self.selected else { return };
        let msgs = self.store.lock().unwrap().nodes[n].messages();
        if msgs.is_empty() {
            return;
        }
        let mut i = msg_index(&msgs, self.msg_sel) as isize;
        match k.code {
            KeyCode::Up => i -= 1,
            KeyCode::Down => i += 1,
            KeyCode::Home | KeyCode::PageUp => i = 0,
            KeyCode::End | KeyCode::PageDown => i = msgs.len() as isize - 1,
            KeyCode::Left | KeyCode::Esc | KeyCode::Tab => {
                self.focus = Focus::Tree;
                self.msg_sel = None;
                return;
            }
            KeyCode::Enter => {
                self.open_viewer(n, self.msg_sel);
                return;
            }
            _ => {}
        }
        let i = i.clamp(0, msgs.len() as isize - 1) as usize;
        self.msg_sel = Some(msgs[i].seq);
    }

    fn open_viewer(&mut self, n: usize, sel: Option<u64>) {
        let (msgs, path) = {
            let st = self.store.lock().unwrap();
            (st.nodes[n].messages(), st.nodes[n].path.clone())
        };
        if msgs.is_empty() {
            return;
        }
        let idx = msg_index(&msgs, sel);
        let mut v = Viewer::new(path, msgs, idx);
        v.relayout(self.width.saturating_sub(4) as usize, &self.th);
        self.viewer = Some(v);
    }

    fn viewer_key(&mut self, k: KeyEvent) {
        let body_h = self.viewer_body_h();
        let v = self.viewer.as_mut().unwrap();
        if v.key(k, body_h, &self.th) {
            self.viewer = None;
        }
    }

    pub fn viewer_body_h(&self) -> usize {
        match &self.viewer {
            Some(v) => (self.height as usize).saturating_sub(3 + v.head.len()).max(1),
            None => 0,
        }
    }
}

/// Finds `sel` in `msgs`, falling back to the oldest if it rotated out.
pub fn msg_index(msgs: &[Arc<Message>], sel: Option<u64>) -> usize {
    let Some(sel) = sel else { return 0 };
    msgs.iter()
        .position(|m| m.seq == sel)
        .or_else(|| msgs.iter().position(|m| m.seq < sel))
        .unwrap_or(msgs.len().saturating_sub(1))
}

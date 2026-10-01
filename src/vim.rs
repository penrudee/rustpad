//! A small Vim emulation on top of tui-textarea.
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_textarea::{CursorMove, Scrolling, TextArea};

type Ta = TextArea<'static>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    VisualLine,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::VisualLine => "V-LINE",
        }
    }
}

pub enum Act {
    None,
    Command,
    Search,
}

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Excl,
    Incl,
    Line,
}

pub struct Vim {
    pub mode: Mode,
    op: Option<char>,
    g: bool,
    count: usize,
    opn: usize,
    reg: String,
    linewise: bool,
    anchor: (usize, usize),
}

fn jump(ta: &mut Ta, r: usize, c: usize) {
    ta.move_cursor(CursorMove::Jump(r.min(u16::MAX as usize) as u16, c.min(u16::MAX as usize) as u16));
}
fn line_len(ta: &Ta, r: usize) -> usize {
    ta.lines().get(r).map_or(0, |l| l.chars().count())
}
fn rep(ta: &mut Ta, m: CursorMove, n: usize) {
    for _ in 0..n {
        ta.move_cursor(m);
    }
}
fn first_nonblank(ta: &mut Ta) {
    let r = ta.cursor().0;
    let k = ta.lines()[r].chars().take_while(|c| c.is_whitespace()).count();
    jump(ta, r, k);
}

impl Vim {
    pub fn new() -> Self {
        Self { mode: Mode::Normal, op: None, g: false, count: 0, opn: 0, reg: String::new(), linewise: false, anchor: (0, 0) }
    }

    pub fn pending(&self) -> String {
        let mut s = String::new();
        if self.opn > 0 {
            s += &self.opn.to_string();
        }
        if let Some(o) = self.op {
            s.push(o);
        }
        if self.count > 0 {
            s += &self.count.to_string();
        }
        if self.g {
            s.push('g');
        }
        s
    }

    fn reset(&mut self) {
        self.op = None;
        self.g = false;
        self.count = 0;
        self.opn = 0;
    }

    pub fn key(&mut self, ta: &mut Ta, k: KeyEvent) -> Act {
        if self.mode == Mode::Insert {
            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            if k.code == KeyCode::Esc || (ctrl && matches!(k.code, KeyCode::Char('c') | KeyCode::Char('['))) {
                self.mode = Mode::Normal;
                if ta.cursor().1 > 0 {
                    ta.move_cursor(CursorMove::Back);
                }
            } else {
                ta.input(k);
            }
            Act::None
        } else {
            self.normal(ta, k)
        }
    }

    // ---- motions -------------------------------------------------------
    fn motion(&mut self, ta: &mut Ta, c: char, n: usize, gpre: bool, had_count: bool) -> Option<Kind> {
        use CursorMove::*;
        Some(match (gpre, c) {
            (false, 'h') => { rep(ta, Back, n); Kind::Excl }
            (false, 'l') => { rep(ta, Forward, n); Kind::Excl }
            (false, 'j') => { rep(ta, Down, n); Kind::Line }
            (false, 'k') => { rep(ta, Up, n); Kind::Line }
            (false, 'w') => { rep(ta, WordForward, n); Kind::Excl }
            (false, 'b') => { rep(ta, WordBack, n); Kind::Excl }
            (false, 'e') => { rep(ta, WordEnd, n); Kind::Incl }
            (false, '0') => { ta.move_cursor(Head); Kind::Excl }
            (false, '^') => { first_nonblank(ta); Kind::Excl }
            (false, '$') => { ta.move_cursor(End); Kind::Excl }
            (false, '{') => { rep(ta, ParagraphBack, n); Kind::Excl }
            (false, '}') => { rep(ta, ParagraphForward, n); Kind::Excl }
            (false, 'G') => {
                if had_count { jump(ta, n - 1, 0) } else { ta.move_cursor(Bottom); ta.move_cursor(Head) }
                Kind::Line
            }
            (true, 'g') => { ta.move_cursor(Top); ta.move_cursor(Head); Kind::Line }
            _ => return None,
        })
    }

    // ---- operators -----------------------------------------------------
    fn line_op(&mut self, ta: &mut Ta, op: char, s: usize, e: usize) {
        let last = ta.lines().len() - 1;
        let (s, e) = (s.min(last), e.min(last));
        self.reg = ta.lines()[s..=e].join("\n");
        self.linewise = true;
        match op {
            'y' => {
                let col = ta.cursor().1;
                jump(ta, s, col);
            }
            'd' => {
                let total = ta.lines().len();
                if e + 1 < total {
                    jump(ta, s, 0);
                    ta.start_selection();
                    jump(ta, e + 1, 0);
                } else if s > 0 {
                    jump(ta, s - 1, line_len(ta, s - 1));
                    ta.start_selection();
                    jump(ta, e, line_len(ta, e));
                } else {
                    jump(ta, 0, 0);
                    ta.start_selection();
                    jump(ta, e, line_len(ta, e));
                }
                ta.cut();
                let r = ta.cursor().0;
                jump(ta, r, 0);
            }
            _ => {
                jump(ta, s, 0);
                ta.start_selection();
                jump(ta, e, line_len(ta, e));
                ta.cut();
                self.mode = Mode::Insert;
            }
        }
    }

    fn range_op(&mut self, ta: &mut Ta, op: char, a: (usize, usize), b: (usize, usize)) {
        if a == b {
            return;
        }
        jump(ta, a.0, a.1);
        ta.start_selection();
        jump(ta, b.0, b.1);
        if op == 'y' {
            ta.copy();
            ta.cancel_selection();
            jump(ta, a.0, a.1);
        } else {
            ta.cut();
        }
        self.reg = ta.yank_text();
        self.linewise = false;
        if op == 'c' {
            self.mode = Mode::Insert;
        }
    }

    fn apply_op(&mut self, ta: &mut Ta, op: char, start: (usize, usize), end: (usize, usize), kind: Kind) {
        match kind {
            Kind::Line => self.line_op(ta, op, start.0.min(end.0), start.0.max(end.0)),
            _ => {
                let (a, mut b) = if start <= end { (start, end) } else { (end, start) };
                if kind == Kind::Incl {
                    b.1 = (b.1 + 1).min(line_len(ta, b.0));
                }
                self.range_op(ta, op, a, b);
            }
        }
    }

    fn visual_op(&mut self, ta: &mut Ta, c: char) {
        let cur = ta.cursor();
        let (a, b) = if self.anchor <= cur { (self.anchor, cur) } else { (cur, self.anchor) };
        ta.cancel_selection();
        let op = match c {
            'y' => 'y',
            'c' | 's' => 'c',
            _ => 'd',
        };
        let linewise = self.mode == Mode::VisualLine;
        self.mode = Mode::Normal;
        if linewise {
            self.line_op(ta, op, a.0, b.0);
        } else {
            let end = (b.0, (b.1 + 1).min(line_len(ta, b.0)));
            self.range_op(ta, op, a, end);
        }
    }

    fn refresh_vline(&self, ta: &mut Ta) {
        let cur = ta.cursor().0;
        ta.cancel_selection();
        if cur >= self.anchor.0 {
            jump(ta, self.anchor.0, 0);
            ta.start_selection();
            jump(ta, cur, line_len(ta, cur));
        } else {
            jump(ta, self.anchor.0, line_len(ta, self.anchor.0));
            ta.start_selection();
            jump(ta, cur, 0);
        }
    }

    fn paste(&mut self, ta: &mut Ta, after: bool) {
        if self.reg.is_empty() {
            return;
        }
        let reg = self.reg.clone();
        let nl = reg.matches('\n').count();
        if self.linewise {
            if after {
                ta.move_cursor(CursorMove::End);
                ta.insert_newline();
                ta.insert_str(&reg);
                rep(ta, CursorMove::Up, nl);
            } else {
                ta.move_cursor(CursorMove::Head);
                ta.insert_str(format!("{reg}\n"));
                rep(ta, CursorMove::Up, nl + 1);
            }
            ta.move_cursor(CursorMove::Head);
        } else {
            let (r, c) = ta.cursor();
            if after && c < line_len(ta, r) {
                ta.move_cursor(CursorMove::Forward);
            }
            ta.insert_str(&reg);
            ta.move_cursor(CursorMove::Back);
        }
    }

    // ---- main normal/visual handler -------------------------------------
    fn normal(&mut self, ta: &mut Ta, k: KeyEvent) -> Act {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let visual = self.mode != Mode::Normal;
        let n0 = self.count.max(1);

        if k.code == KeyCode::Esc || (ctrl && matches!(k.code, KeyCode::Char('c') | KeyCode::Char('['))) {
            self.reset();
            if visual {
                ta.cancel_selection();
                self.mode = Mode::Normal;
            }
            return Act::None;
        }
        if ctrl {
            match k.code {
                KeyCode::Char('r') => for _ in 0..n0 { ta.redo(); },
                KeyCode::Char('d') => ta.scroll(Scrolling::HalfPageDown),
                KeyCode::Char('u') => ta.scroll(Scrolling::HalfPageUp),
                KeyCode::Char('f') => ta.scroll(Scrolling::PageDown),
                KeyCode::Char('b') => ta.scroll(Scrolling::PageUp),
                _ => {}
            }
            self.count = 0;
            return Act::None;
        }

        let c = match k.code {
            KeyCode::Char(c) => c,
            KeyCode::Left | KeyCode::Backspace => 'h',
            KeyCode::Right => 'l',
            KeyCode::Up => 'k',
            KeyCode::Down | KeyCode::Enter => 'j',
            KeyCode::Home => '^',
            KeyCode::End => '$',
            KeyCode::Delete => 'x',
            KeyCode::PageDown => { ta.scroll(Scrolling::PageDown); return Act::None; }
            KeyCode::PageUp => { ta.scroll(Scrolling::PageUp); return Act::None; }
            _ => return Act::None,
        };

        // r{char}
        if self.op == Some('r') {
            if !visual && matches!(k.code, KeyCode::Char(_)) {
                for _ in 0..n0 {
                    ta.delete_next_char();
                    ta.insert_char(c);
                }
                ta.move_cursor(CursorMove::Back);
            }
            self.reset();
            return Act::None;
        }

        // counts
        if c.is_ascii_digit() && !(c == '0' && self.count == 0) && matches!(k.code, KeyCode::Char(_)) {
            self.count = self.count * 10 + c.to_digit(10).unwrap() as usize;
            return Act::None;
        }
        let n = n0 * self.opn.max(1);
        let had_count = self.count > 0 || self.opn > 0;

        // gg prefix
        if c == 'g' && !self.g {
            self.g = true;
            return Act::None;
        }
        let gpre = std::mem::take(&mut self.g);

        // operator-pending (d / y / c + motion)
        if let Some(op) = self.op.filter(|o| matches!(o, 'd' | 'y' | 'c')) {
            if !visual {
                let row = ta.cursor().0;
                if c == op {
                    let last = ta.lines().len() - 1;
                    self.line_op(ta, op, row, (row + n - 1).min(last));
                } else {
                    let start = ta.cursor();
                    let mc = if op == 'c' && c == 'w' { 'e' } else { c };
                    if let Some(kind) = self.motion(ta, mc, n, gpre, had_count) {
                        let mut end = ta.cursor();
                        if mc == 'w' && end.0 > start.0 {
                            end = (start.0, line_len(ta, start.0));
                        }
                        self.apply_op(ta, op, start, end, kind);
                    }
                }
                self.reset();
                self.clamp(ta);
                return Act::None;
            }
        }

        // visual-mode operators
        if visual {
            match c {
                'y' | 'd' | 'x' | 'c' | 's' | 'X' | 'D' => {
                    self.visual_op(ta, c);
                    self.reset();
                    self.clamp(ta);
                    return Act::None;
                }
                'v' | 'V' => {
                    ta.cancel_selection();
                    self.mode = Mode::Normal;
                    self.reset();
                    return Act::None;
                }
                ':' => {
                    ta.cancel_selection();
                    self.mode = Mode::Normal;
                    self.reset();
                    return Act::Command;
                }
                _ => {}
            }
        }

        // plain motion
        if self.motion(ta, c, n, gpre, had_count).is_some() {
            if self.mode == Mode::VisualLine {
                self.refresh_vline(ta);
            }
            self.reset();
            self.clamp(ta);
            return Act::None;
        }

        // commands
        use CursorMove::*;
        let (row, col) = ta.cursor();
        let mut act = Act::None;
        match c {
            'i' => self.mode = Mode::Insert,
            'a' => {
                if col < line_len(ta, row) { ta.move_cursor(Forward); }
                self.mode = Mode::Insert;
            }
            'I' => { first_nonblank(ta); self.mode = Mode::Insert; }
            'A' => { ta.move_cursor(End); self.mode = Mode::Insert; }
            'o' => { ta.move_cursor(End); ta.insert_newline(); self.mode = Mode::Insert; }
            'O' => { ta.move_cursor(Head); ta.insert_newline(); ta.move_cursor(Up); self.mode = Mode::Insert; }
            'x' => {
                let mut got = String::new();
                for _ in 0..n {
                    let (r, c) = ta.cursor();
                    if let Some(ch) = ta.lines()[r].chars().nth(c) { got.push(ch); ta.delete_next_char(); }
                }
                if !got.is_empty() { self.reg = got; self.linewise = false; }
            }
            'X' => for _ in 0..n { ta.delete_char(); },
            'D' => { ta.delete_line_by_end(); }
            'C' => { ta.delete_line_by_end(); self.mode = Mode::Insert; }
            's' => { ta.delete_next_char(); self.mode = Mode::Insert; }
            'S' => self.line_op(ta, 'c', row, row),
            'Y' => self.line_op(ta, 'y', row, row),
            'd' | 'y' | 'c' => {
                self.op = Some(c);
                self.opn = self.count;
                self.count = 0;
                return Act::None;
            }
            'r' => { self.op = Some('r'); return Act::None; }
            'p' => for _ in 0..n { self.paste(ta, true); },
            'P' => for _ in 0..n { self.paste(ta, false); },
            'u' => for _ in 0..n { ta.undo(); },
            'J' => {
                ta.move_cursor(End);
                if row + 1 < ta.lines().len() {
                    ta.delete_next_char();
                    ta.insert_char(' ');
                    ta.move_cursor(Back);
                }
            }
            'v' => { self.anchor = ta.cursor(); self.mode = Mode::Visual; ta.start_selection(); }
            'V' => { self.anchor = ta.cursor(); self.mode = Mode::VisualLine; self.refresh_vline(ta); }
            ':' => act = Act::Command,
            '/' => act = Act::Search,
            'n' => { ta.search_forward(false); }
            'N' => { ta.search_back(false); }
            _ => {}
        }
        self.reset();
        self.clamp(ta);
        act
    }

    /// In Normal mode the cursor sits *on* a character, never after the last one.
    fn clamp(&self, ta: &mut Ta) {
        if self.mode == Mode::Normal {
            let (r, c) = ta.cursor();
            let l = line_len(ta, r);
            if l > 0 && c >= l {
                ta.move_cursor(CursorMove::Back);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, keys: &str) -> (Vec<String>, Vim) {
        let mut ta: Ta = TextArea::new(text.lines().map(String::from).collect());
        let mut v = Vim::new();
        for c in keys.chars() {
            let k = if c == '\x1b' { KeyCode::Esc } else { KeyCode::Char(c) };
            v.key(&mut ta, KeyEvent::new(k, KeyModifiers::NONE));
        }
        (ta.lines().to_vec(), v)
    }

    #[test]
    fn dd_and_paste() {
        assert_eq!(run("a\nb\nc", "dd").0, ["b", "c"]);
        assert_eq!(run("a\nb\nc", "jdd").0, ["a", "c"]);
        assert_eq!(run("a\nb\nc", "jjdd").0, ["a", "b"]);
        assert_eq!(run("a\nb\nc", "yyjp").0, ["a", "b", "a", "c"]);
        assert_eq!(run("a\nb\nc", "yyP").0, ["a", "a", "b", "c"]);
        assert_eq!(run("a\nb\nc\nd", "2dd").0, ["c", "d"]);
    }

    #[test]
    fn word_ops() {
        assert_eq!(run("foo bar baz", "dw").0, ["bar baz"]);
        assert_eq!(run("foo bar baz", "wdw").0, ["foo baz"]);
        assert_eq!(run("foo bar baz", "cwxx\x1b").0, ["xx bar baz"]);
        assert_eq!(run("foo bar baz", "wD").0, ["foo "]);
        assert_eq!(run("foo bar baz", "d$").0, [""]);
        assert_eq!(run("foo bar", "yw$p").0, ["foo barfoo "]);
    }

    #[test]
    fn insert_and_misc() {
        assert_eq!(run("ab", "Axy\x1b").0, ["abxy"]);
        assert_eq!(run("ab", "ohi\x1b").0, ["ab", "hi"]);
        assert_eq!(run("abc", "xp").0, ["bac"]);
        assert_eq!(run("abc", "xu").0, ["abc"]);
        assert_eq!(run("abc", "rz").0, ["zbc"]);
        assert_eq!(run("a\nb", "J").0, ["a b"]);
        assert_eq!(run("a\nb\nc", "dG").0, [""]);
        assert_eq!(run("a\nb\nc", "jdgg").0, ["c"]);
    }

    #[test]
    fn visual() {
        assert_eq!(run("hello world", "vlld").0, ["lo world"]);
        assert_eq!(run("hello world", "vey$p").0, ["hello worldhello"]);
        assert_eq!(run("a\nb\nc", "Vjd").0, ["c"]);
        assert_eq!(run("a\nb\nc", "Vjyjjp").0, ["a", "b", "c", "a", "b"]);
    }
}

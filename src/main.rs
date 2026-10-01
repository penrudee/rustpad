mod md;
mod store;
mod vim;

use anyhow::Result;
use chrono::Local;
use ratatui::{
    crossterm::{
        event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
        execute,
    },
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap},
    DefaultTerminal, Frame,
};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::stdout,
    path::PathBuf,
    time::{Duration, Instant},
};
use store::Store;
use tui_textarea::{CursorMove, TextArea};
use vim::{Act, Mode, Vim};

const AUTOSAVE_AFTER: Duration = Duration::from_millis(1200);

const WELCOME: &str = "# Welcome to rustpad

A tiny markdown notepad with **Vim keys**.

- Press `i` to write, `Esc` to go back to NORMAL mode
- Notes **autosave** ~1 second after you stop typing
- `Ctrl-p` toggles the live preview, `:help` shows all keys
- `:backup` makes a zip, `:restore latest` brings it back

> Mouse selection / copy / paste in your terminal works as usual.

```rust
fn main() { println!(\"hello\"); }
```
";

const HELP: &str = "\
 MODES      i a I A o O  insert      v V  visual / visual-line      Esc  normal
 MOTION     h j k l  w b e  0 ^ $  gg G  {n}G  { }  Ctrl-d/u/f/b     (counts: 5j, 3w)
 EDIT       x X D C s S  r{c}  J  u  Ctrl-r      p P  paste
 OPERATORS  d y c + motion   dd yy cc  dw d$ y3w cw  dG dgg      (visual: y d c)
 SEARCH     /pattern  n  N

 FOCUS      Tab  switch editor <-> note list      F2  toggle list      F3 / Ctrl-p  preview
 LIST       j k  move   Enter  open   n  new   r  rename   x  delete
 SAVE       Ctrl-s or :w   (autosave runs on its own)

 :w  :q  :q!  :wq        :new NAME   :open NAME   :rename NAME   :rm  :rm!
 :backup [file.zip]      :restore <file.zip|latest>      :preview   :list   :{line}
 :help

 Mouse: not captured -> select/copy with the mouse and paste (Ctrl-Shift-V) work normally.
 Press any key to close.";

#[derive(PartialEq, Clone, Copy)]
enum Focus {
    Editor,
    List,
}

struct App {
    store: Store,
    notes: Vec<String>,
    list: ListState,
    current: String,
    ta: TextArea<'static>,
    vim: Vim,
    focus: Focus,
    show_list: bool,
    show_preview: bool,
    show_help: bool,
    dirty: bool,
    last_edit: Instant,
    saved_at: Option<chrono::DateTime<Local>>,
    cmd: Option<String>,
    status: String,
    quit: bool,
    preview: Text<'static>,
    preview_dirty: bool,
    preview_w: usize,
    preview_rows: usize,
}

fn fingerprint(ta: &TextArea) -> u64 {
    let mut h = DefaultHasher::new();
    ta.lines().len().hash(&mut h);
    for l in ta.lines() {
        l.hash(&mut h);
    }
    h.finish()
}

fn make_ta(text: &str) -> TextArea<'static> {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    if lines.is_empty() {
        lines.push(String::new());
    }
    let mut ta = TextArea::new(lines);
    ta.set_tab_length(2);
    ta.set_line_number_style(Style::new().fg(Color::DarkGray));
    ta.set_cursor_line_style(Style::new().bg(Color::Indexed(236)));
    ta.set_selection_style(Style::new().bg(Color::Blue));
    ta.set_search_style(Style::new().bg(Color::Yellow).fg(Color::Black));
    ta
}

impl App {
    fn new(store: Store, open: Option<String>) -> Result<Self> {
        let mut notes = store.list()?;
        if notes.is_empty() {
            store.save("welcome", WELCOME)?;
            notes = store.list()?;
        }
        let mut app = App {
            store,
            notes,
            list: ListState::default(),
            current: String::new(),
            ta: make_ta(""),
            vim: Vim::new(),
            focus: Focus::Editor,
            show_list: true,
            show_preview: true,
            show_help: false,
            dirty: false,
            last_edit: Instant::now(),
            saved_at: None,
            cmd: None,
            status: "Press F1 or :help for keys".into(),
            quit: false,
            preview: Text::default(),
            preview_dirty: true,
            preview_w: 0,
            preview_rows: 0,
        };
        let first = match open {
            Some(n) => {
                let n = Store::sanitize(&n)?;
                if !app.store.exists(&n) {
                    app.store.save(&n, &format!("# {n}\n\n"))?;
                    app.notes = app.store.list()?;
                }
                n
            }
            None => app.notes[0].clone(),
        };
        app.open(&first)?;
        Ok(app)
    }

    fn msg(&mut self, s: impl Into<String>) {
        self.status = s.into();
    }

    fn refresh_list(&mut self) {
        self.notes = self.store.list().unwrap_or_default();
        let idx = self.notes.iter().position(|n| *n == self.current).unwrap_or(0);
        self.list.select(Some(idx));
    }

    fn save(&mut self, quiet: bool) {
        if self.current.is_empty() {
            return;
        }
        let mut text = self.ta.lines().join("\n");
        text.push('\n');
        match self.store.save(&self.current, &text) {
            Ok(()) => {
                self.dirty = false;
                self.saved_at = Some(Local::now());
                if !quiet {
                    self.msg(format!("saved {}.md", self.current));
                }
            }
            Err(e) => self.msg(format!("SAVE FAILED: {e}")),
        }
    }

    fn open(&mut self, name: &str) -> Result<()> {
        if self.dirty {
            self.save(true);
        }
        let text = self.store.load(name)?;
        self.ta = make_ta(&text);
        self.vim = Vim::new();
        self.current = name.to_string();
        self.dirty = false;
        self.preview_dirty = true;
        self.refresh_list();
        Ok(())
    }

    fn tick(&mut self) {
        if self.dirty && self.last_edit.elapsed() >= AUTOSAVE_AFTER {
            self.save(true);
            self.msg("autosaved");
        }
    }

    fn touched(&mut self) {
        self.dirty = true;
        self.preview_dirty = true;
        self.last_edit = Instant::now();
    }

    // ---------------------------------------------------------------- input
    fn on_paste(&mut self, s: String) {
        let s = s.replace('\r', "");
        if let Some(c) = &mut self.cmd {
            c.push_str(&s.replace('\n', " "));
        } else if self.focus == Focus::Editor && !self.show_help {
            if self.ta.insert_str(s) {
                self.touched();
            }
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        if self.show_help {
            self.show_help = false;
            return;
        }
        if self.cmd.is_some() {
            return self.cmdline_key(k);
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('s') if ctrl => return self.save(false),
            KeyCode::Char('p') if ctrl => return self.show_preview = !self.show_preview,
            KeyCode::F(1) => return self.show_help = true,
            KeyCode::F(2) => return self.show_list = !self.show_list,
            KeyCode::F(3) => return self.show_preview = !self.show_preview,
            _ => {}
        }
        if self.focus == Focus::List {
            return self.list_key(k);
        }
        if self.vim.mode == Mode::Normal && k.code == KeyCode::Tab {
            self.show_list = true;
            self.focus = Focus::List;
            return;
        }
        let before = fingerprint(&self.ta);
        match self.vim.key(&mut self.ta, k) {
            Act::Command => self.cmd = Some(":".into()),
            Act::Search => self.cmd = Some("/".into()),
            Act::None => {}
        }
        if fingerprint(&self.ta) != before {
            self.touched();
        }
    }

    fn list_key(&mut self, k: KeyEvent) {
        let sel = self.list.selected().unwrap_or(0);
        let len = self.notes.len();
        match k.code {
            KeyCode::Tab | KeyCode::Esc => self.focus = Focus::Editor,
            KeyCode::Char('j') | KeyCode::Down => self.list.select(Some((sel + 1).min(len.saturating_sub(1)))),
            KeyCode::Char('k') | KeyCode::Up => self.list.select(Some(sel.saturating_sub(1))),
            KeyCode::Char('g') => self.list.select(Some(0)),
            KeyCode::Char('G') => self.list.select(Some(len.saturating_sub(1))),
            KeyCode::Enter | KeyCode::Char('l') => {
                if let Some(n) = self.notes.get(sel).cloned() {
                    if let Err(e) = self.open(&n) {
                        self.msg(format!("open failed: {e}"));
                    }
                    self.focus = Focus::Editor;
                }
            }
            KeyCode::Char('n') => self.cmd = Some(":new ".into()),
            KeyCode::Char('r') => self.cmd = Some(":rename ".into()),
            KeyCode::Char('x') => self.cmd = Some(":rm".into()),
            KeyCode::Char(':') => self.cmd = Some(":".into()),
            _ => {}
        }
    }

    fn cmdline_key(&mut self, k: KeyEvent) {
        let Some(c) = self.cmd.as_mut() else { return };
        match k.code {
            KeyCode::Esc => self.cmd = None,
            KeyCode::Backspace => {
                c.pop();
                if c.is_empty() {
                    self.cmd = None;
                }
            }
            KeyCode::Char(ch) if !k.modifiers.contains(KeyModifiers::CONTROL) => c.push(ch),
            KeyCode::Char('c') => self.cmd = None,
            KeyCode::Enter => {
                let line = self.cmd.take().unwrap_or_default();
                if let Some(pat) = line.strip_prefix('/') {
                    match self.ta.set_search_pattern(pat) {
                        Ok(()) => {
                            if !self.ta.search_forward(false) {
                                self.msg("pattern not found");
                            }
                        }
                        Err(e) => self.msg(format!("bad pattern: {e}")),
                    }
                } else if let Some(cmd) = line.strip_prefix(':') {
                    self.exec(cmd.trim());
                }
            }
            _ => {}
        }
    }

    fn exec(&mut self, line: &str) {
        let (cmd, arg) = match line.split_once(char::is_whitespace) {
            Some((c, a)) => (c, a.trim()),
            None => (line, ""),
        };
        let res: Result<()> = (|| {
            match cmd {
                "" => {}
                "w" | "write" => self.save(false),
                "wq" | "x" => {
                    self.save(true);
                    self.quit = true;
                }
                "q" | "quit" => {
                    self.save(true); // autosave philosophy: never lose text
                    self.quit = true;
                }
                "q!" => self.quit = true,
                "preview" | "pv" => self.show_preview = !self.show_preview,
                "list" | "ls" => self.show_list = !self.show_list,
                "help" | "h" => self.show_help = true,
                "new" | "e" | "open" => {
                    let name = Store::sanitize(arg)?;
                    if !self.store.exists(&name) {
                        if self.dirty {
                            self.save(true);
                        }
                        self.store.save(&name, &format!("# {name}\n\n"))?;
                    }
                    self.open(&name)?;
                    if cmd == "new" {
                        self.ta.move_cursor(CursorMove::Bottom);
                        self.ta.move_cursor(CursorMove::End);
                        self.vim.mode = Mode::Insert;
                    }
                    self.focus = Focus::Editor;
                    self.msg(format!("opened {name}.md"));
                }
                "rename" | "mv" => {
                    self.save(true);
                    let new = Store::sanitize(arg)?;
                    self.store.rename(&self.current, &new)?;
                    self.current = new.clone();
                    self.refresh_list();
                    self.msg(format!("renamed to {new}.md"));
                }
                "rm" => self.msg(format!("delete '{}'? run :rm! to confirm (moved to trash/)", self.current)),
                "rm!" => {
                    let old = self.current.clone();
                    self.dirty = false;
                    self.store.trash(&old)?;
                    self.notes = self.store.list()?;
                    if self.notes.is_empty() {
                        self.store.save("untitled", "# untitled\n\n")?;
                        self.notes = self.store.list()?;
                    }
                    let first = self.notes[0].clone();
                    self.open(&first)?;
                    self.msg(format!("deleted {old}.md"));
                }
                "backup" => {
                    self.save(true);
                    let out = (!arg.is_empty()).then(|| PathBuf::from(arg));
                    let p = self.store.backup(out, "backup")?;
                    self.msg(format!("backup -> {}", p.display()));
                }
                "restore" => {
                    let path = match arg {
                        "" => {
                            self.msg(format!("usage: :restore <file.zip|latest>   dir: {}", self.store.backups_dir().display()));
                            return Ok(());
                        }
                        "latest" => self.store.latest_backup().ok_or_else(|| anyhow::anyhow!("no backups yet"))?,
                        p => PathBuf::from(p),
                    };
                    self.dirty = false;
                    let n = self.store.restore(&path)?;
                    self.notes = self.store.list()?;
                    let first = self.notes[0].clone();
                    self.open(&first)?;
                    self.msg(format!("restored {n} notes (previous state saved as pre-restore-*.zip)"));
                }
                n if n.chars().all(|c| c.is_ascii_digit()) => {
                    let n: usize = n.parse()?;
                    self.ta.move_cursor(CursorMove::Jump(n.saturating_sub(1).min(u16::MAX as usize) as u16, 0));
                }
                other => self.msg(format!("unknown command: {other}")),
            }
            Ok(())
        })();
        if let Err(e) = res {
            self.msg(format!("error: {e}"));
        }
    }
}

// ------------------------------------------------------------------- drawing
fn ui(f: &mut Frame, app: &mut App) {
    let [main, bar] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(f.area());
    let mut cons = vec![];
    if app.show_list {
        cons.push(Constraint::Length(24));
    }
    cons.push(Constraint::Fill(1));
    if app.show_preview {
        cons.push(Constraint::Fill(1));
    }
    let cols = Layout::horizontal(cons).split(main);
    let mut i = 0;
    if app.show_list {
        draw_list(f, app, cols[i]);
        i += 1;
    }
    draw_editor(f, app, cols[i]);
    i += 1;
    if app.show_preview {
        draw_preview(f, app, cols[i]);
    }
    draw_bar(f, app, bar);
    if app.show_help {
        let a = f.area();
        let w = 100.min(a.width.saturating_sub(4));
        let h = (HELP.lines().count() as u16 + 2).min(a.height);
        let r = Rect::new(a.x + (a.width.saturating_sub(w)) / 2, a.y + (a.height.saturating_sub(h)) / 2, w, h);
        f.render_widget(Clear, r);
        f.render_widget(
            Paragraph::new(HELP).block(Block::bordered().title(" rustpad help ").border_style(Style::new().fg(Color::Yellow))),
            r,
        );
    }
}

fn border(focused: bool) -> Style {
    Style::new().fg(if focused { Color::Cyan } else { Color::DarkGray })
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app.notes.iter().map(|n| ListItem::new(n.as_str())).collect();
    let list = List::new(items)
        .block(Block::bordered().title(" Notes ").border_style(border(app.focus == Focus::List)))
        .highlight_style(Style::new().bg(Color::Indexed(24)).add_modifier(Modifier::BOLD))
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, area, &mut app.list);
}

fn draw_editor(f: &mut Frame, app: &mut App, area: Rect) {
    let mark = if app.dirty { " ●" } else { "" };
    let title = format!(" {}.md{} ", app.current, mark);
    app.ta.set_block(Block::bordered().title(title).border_style(border(app.focus == Focus::Editor)));
    let cursor = match app.vim.mode {
        Mode::Insert => Style::new().fg(Color::Black).bg(Color::Green),
        _ => Style::new().add_modifier(Modifier::REVERSED),
    };
    app.ta.set_cursor_style(if app.focus == Focus::Editor { cursor } else { Style::default() });
    f.render_widget(&app.ta, area);
}

fn draw_preview(f: &mut Frame, app: &mut App, area: Rect) {
    let w = area.width.saturating_sub(2) as usize;
    if app.preview_dirty || app.preview_w != w {
        app.preview = md::render(&app.ta.lines().join("\n"), w);
        app.preview_w = w;
        app.preview_dirty = false;
        app.preview_rows = app.preview.lines.iter().map(|l| l.width().max(1).div_ceil(w.max(1))).sum();
    }
    let h = area.height.saturating_sub(2) as usize;
    let (row, _) = app.ta.cursor();
    let n = app.ta.lines().len();
    let scroll = if n > 1 { row * app.preview_rows.saturating_sub(h) / (n - 1) } else { 0 };
    let p = Paragraph::new(app.preview.clone())
        .wrap(Wrap { trim: false })
        .scroll((scroll.min(u16::MAX as usize) as u16, 0))
        .block(Block::bordered().title(" Preview ").border_style(border(false)));
    f.render_widget(p, area);
}

fn draw_bar(f: &mut Frame, app: &App, area: Rect) {
    if let Some(c) = &app.cmd {
        f.render_widget(Paragraph::new(c.as_str()), area);
        f.set_cursor_position((area.x + c.chars().count() as u16, area.y));
        return;
    }
    let (label, color) = if app.focus == Focus::List {
        ("LIST", Color::Magenta)
    } else {
        match app.vim.mode {
            Mode::Normal => ("NORMAL", Color::Cyan),
            Mode::Insert => ("INSERT", Color::Green),
            _ => (app.vim.mode.label(), Color::Yellow),
        }
    };
    let (r, c) = app.ta.cursor();
    let save = match (app.dirty, app.saved_at) {
        (true, _) => "unsaved…".to_string(),
        (false, Some(t)) => format!("saved {}", t.format("%H:%M:%S")),
        (false, None) => "saved".to_string(),
    };
    let left = Line::from(vec![
        Span::styled(format!(" {label} "), Style::new().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD)),
        Span::raw(format!(" {}  {}  {}", app.current, save, app.vim.pending())),
        Span::styled(format!("  {}", app.status), Style::new().fg(Color::DarkGray)),
    ]);
    let right = format!("Ln {}, Col {}  ", r + 1, c + 1);
    f.render_widget(Paragraph::new(left), area);
    f.render_widget(Paragraph::new(right).right_aligned(), area);
}

// ---------------------------------------------------------------------- main
fn run(term: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        term.draw(|f| ui(f, app))?;
        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => app.on_key(k),
                Event::Paste(s) => app.on_paste(s),
                _ => {}
            }
        }
        app.tick();
        if app.quit {
            return Ok(());
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let store = Store::open()?;
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") => {
            println!("rustpad [NOTE]            open (or create) a note\nrustpad backup [FILE.zip] zip all notes\nrustpad restore FILE.zip|latest\n\ndata dir: {}", store.root.display());
            return Ok(());
        }
        Some("backup") => {
            let p = store.backup(args.get(1).map(PathBuf::from), "backup")?;
            println!("backup written: {}", p.display());
            return Ok(());
        }
        Some("restore") => {
            let p = match args.get(1).map(String::as_str) {
                Some("latest") | None => store.latest_backup().ok_or_else(|| anyhow::anyhow!("no backups found"))?,
                Some(p) => PathBuf::from(p),
            };
            let n = store.restore(&p)?;
            println!("restored {n} notes from {}", p.display());
            return Ok(());
        }
        _ => {}
    }
    let mut app = App::new(store, args.first().cloned())?;
    let mut term = ratatui::init(); // raw mode + alt screen + panic hook; mouse NOT captured
    execute!(stdout(), EnableBracketedPaste)?;
    let res = run(&mut term, &mut app);
    app.save(true);
    execute!(stdout(), DisableBracketedPaste)?;
    ratatui::restore();
    res
}

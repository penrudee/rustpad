//! Markdown -> ratatui Text (for the live preview pane).
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};

struct R {
    lines: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
    styles: Vec<Style>,
    quote: usize,
    lists: Vec<Option<u64>>,
    pending: Option<String>,
    in_code: bool,
    links: Vec<String>,
    width: usize,
}

impl R {
    fn style(&self) -> Style {
        self.styles.iter().fold(Style::default(), |a, s| a.patch(*s))
    }
    fn push(&mut self, text: String, style: Style) {
        if let Some(p) = self.pending.take() {
            self.cur.push(Span::styled(p, Style::new().fg(Color::Cyan)));
        }
        self.cur.push(Span::styled(text, style));
    }
    fn flush(&mut self) {
        if self.cur.is_empty() {
            return;
        }
        let mut spans = vec![];
        if self.quote > 0 {
            spans.push(Span::styled("▎ ".repeat(self.quote), Style::new().fg(Color::DarkGray)));
        }
        spans.append(&mut self.cur);
        self.lines.push(Line::from(spans));
    }
    fn blank(&mut self) {
        self.flush();
        if self.lines.last().is_some_and(|l| !l.spans.is_empty()) {
            self.lines.push(Line::default());
        }
    }
    fn rule(&mut self) {
        self.flush();
        let w = self.width.max(4);
        self.lines.push(Line::styled("─".repeat(w), Style::new().fg(Color::DarkGray)));
    }
}

pub fn render(src: &str, width: usize) -> Text<'static> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut r = R {
        lines: vec![],
        cur: vec![],
        styles: vec![],
        quote: 0,
        lists: vec![],
        pending: None,
        in_code: false,
        links: vec![],
        width,
    };
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let dim = Style::new().fg(Color::DarkGray);
    let code = Style::new().fg(Color::Yellow).bg(Color::Indexed(236));

    for ev in Parser::new_ext(src, opts) {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    r.blank();
                    let n = level as usize;
                    let color = [Color::Magenta, Color::Cyan, Color::Green, Color::Yellow, Color::Blue, Color::Gray][n.min(6) - 1];
                    r.styles.push(bold.fg(color));
                    r.pending = Some("#".repeat(n) + " ");
                }
                Tag::Strong => r.styles.push(bold),
                Tag::Emphasis => r.styles.push(Style::new().add_modifier(Modifier::ITALIC)),
                Tag::Strikethrough => r.styles.push(Style::new().add_modifier(Modifier::CROSSED_OUT)),
                Tag::BlockQuote(_) => {
                    r.flush();
                    r.quote += 1;
                    r.styles.push(Style::new().fg(Color::Gray).add_modifier(Modifier::ITALIC));
                }
                Tag::CodeBlock(kind) => {
                    r.blank();
                    r.in_code = true;
                    if let CodeBlockKind::Fenced(lang) = kind {
                        if !lang.is_empty() {
                            r.push(format!("╭ {lang}"), dim);
                            r.flush();
                        }
                    }
                }
                Tag::List(start) => {
                    r.flush();
                    r.lists.push(start);
                }
                Tag::Item => {
                    r.flush();
                    let indent = "  ".repeat(r.lists.len().saturating_sub(1));
                    let marker = match r.lists.last_mut() {
                        Some(Some(n)) => {
                            let s = format!("{n}. ");
                            *n += 1;
                            s
                        }
                        _ => "• ".to_string(),
                    };
                    r.pending = Some(indent + &marker);
                }
                Tag::Link { dest_url, .. } => {
                    r.styles.push(Style::new().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED));
                    r.links.push(dest_url.to_string());
                }
                Tag::Image { dest_url, .. } => {
                    r.push("[img] ".into(), dim);
                    r.styles.push(Style::new().fg(Color::Cyan));
                    r.links.push(dest_url.to_string());
                }
                Tag::TableHead => r.styles.push(bold),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => {
                    r.styles.pop();
                    r.blank();
                }
                TagEnd::Paragraph => r.blank(),
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                    r.styles.pop();
                }
                TagEnd::BlockQuote(_) => {
                    r.flush();
                    r.styles.pop();
                    r.quote = r.quote.saturating_sub(1);
                    r.blank();
                }
                TagEnd::CodeBlock => {
                    r.in_code = false;
                    r.blank();
                }
                TagEnd::List(_) => {
                    r.lists.pop();
                    if r.lists.is_empty() {
                        r.blank();
                    }
                }
                TagEnd::Item => r.flush(),
                TagEnd::Link | TagEnd::Image => {
                    r.styles.pop();
                    if let Some(u) = r.links.pop() {
                        r.push(format!(" ({u})"), dim);
                    }
                }
                TagEnd::TableCell => r.push(" │ ".into(), dim),
                TagEnd::TableHead => {
                    r.styles.pop();
                    r.flush();
                    r.rule();
                }
                TagEnd::TableRow => r.flush(),
                TagEnd::Table => r.blank(),
                _ => {}
            },
            Event::Text(t) => {
                if r.in_code {
                    for l in t.lines() {
                        r.push(format!("  {l} "), code);
                        r.flush();
                    }
                } else {
                    let st = r.style();
                    r.push(t.to_string(), st);
                }
            }
            Event::Code(c) => r.push(format!(" {c} "), code),
            Event::SoftBreak => {
                let st = r.style();
                r.push(" ".into(), st);
            }
            Event::HardBreak => r.flush(),
            Event::Rule => {
                r.blank();
                r.rule();
                r.blank();
            }
            Event::TaskListMarker(done) => {
                let (s, c) = if done { ("[x] ", Color::Green) } else { ("[ ] ", Color::Yellow) };
                r.push(s.into(), Style::new().fg(c));
            }
            Event::Html(h) | Event::InlineHtml(h) => r.push(h.to_string(), dim),
            _ => {}
        }
    }
    r.flush();
    Text::from(r.lines)
}

// ============================================================
// RATATUI CHAT — Stage 1-3: TUI Shell + Input Handling
// ============================================================
// CONCEPTS COVERED:
//   • Ratatui's render loop (draw → handle events → repeat)
//   • Layout system (constraints, directions)
//   • Widgets: Block, Paragraph, List
//   • Raw mode & the alternate screen
//   • Async event handling with crossterm + tokio

use std::io;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

// ── App State ────────────────────────────────────────────────
// Everything your app needs lives here. Ratatui is immediate-mode:
// you re-draw the entire screen every frame from this state.

#[derive(Debug, Clone, PartialEq)]
enum Role {
    User,
    Assistant,
    System, // for status messages like "Thinking..."
}

#[derive(Debug, Clone)]
struct Message {
    role: Role,
    content: String,
}

struct App {
    messages: Vec<Message>,
    input: String,          // current text in the input box
    input_cursor: usize,    // cursor position (byte index)
    scroll_offset: usize,   // how many lines we've scrolled up
    is_thinking: bool,      // true while waiting for OpenAI
}

impl App {
    fn new() -> Self {
        Self {
            messages: vec![Message {
                role: Role::System,
                content: "Welcome! Type a message and press Enter. Press Ctrl+C to quit.".into(),
            }],
            input: String::new(),
            input_cursor: 0,
            scroll_offset: 0,
            is_thinking: false,
        }
    }

    // Insert a character at the cursor position
    fn insert_char(&mut self, c: char) {
        self.input.insert(self.input_cursor, c);
        self.input_cursor += c.len_utf8();
    }

    // Delete the character before the cursor (backspace)
    fn delete_char(&mut self) {
        if self.input_cursor > 0 {
            // Find the start of the previous char (handles multi-byte UTF-8)
            let prev = self.input[..self.input_cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.remove(prev);
            self.input_cursor = prev;
        }
    }

    fn submit(&mut self) -> Option<String> {
        let text = self.input.trim().to_string();
        if text.is_empty() || self.is_thinking {
            return None;
        }
        self.messages.push(Message { role: Role::User, content: text.clone() });
        self.input.clear();
        self.input_cursor = 0;
        self.scroll_offset = 0; // snap to bottom on send
        Some(text)
    }
}

// ── Drawing ──────────────────────────────────────────────────
// Ratatui's draw() gives you a Frame. You describe what to render
// using widgets — nothing is actually printed until draw() flushes.

fn draw(frame: &mut Frame, app: &App) {
    let size = frame.size();

    // LAYOUT: split the screen into chat area (top) + input box (bottom)
    // Constraints::Min(0) = take all remaining space
    // Constraints::Length(3) = exactly 3 rows tall
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(size);

    draw_messages(frame, app, chunks[0]);
    draw_input(frame, app, chunks[1]);
}

fn draw_messages(frame: &mut Frame, app: &App, area: Rect) {
    // Convert our messages into Ratatui `Line` structs.
    // A Line is a row of styled Spans. Spans are styled text fragments.
    let mut lines: Vec<Line> = Vec::new();

    for msg in &app.messages {
        match msg.role {
            Role::User => {
                // "You › " prefix in cyan, then the message
                lines.push(Line::from(vec![
                    Span::styled("You  › ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw(&msg.content),
                ]));
            }
            Role::Assistant => {
                lines.push(Line::from(vec![
                    Span::styled("AI   › ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw(&msg.content),
                ]));
            }
            Role::System => {
                lines.push(Line::from(Span::styled(
                    format!("     ℹ  {}", msg.content),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        lines.push(Line::from("")); // blank line between messages
    }

    if app.is_thinking {
        lines.push(Line::from(Span::styled(
            "AI   › ▋ thinking...",
            Style::default().fg(Color::Green).add_modifier(Modifier::DIM),
        )));
    }

    // Scroll: calculate how many lines fit and clamp offset
    let visible_height = area.height.saturating_sub(2) as usize; // minus borders
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll = (max_scroll.saturating_sub(app.scroll_offset)) as u16;

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" ratatui-chat "))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, area);
}

fn draw_input(frame: &mut Frame, app: &App, area: Rect) {
    let label = if app.is_thinking { " waiting... " } else { " message " };

    let input_style = if app.is_thinking {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };

    // Show cursor as a block character appended at cursor position
    let display = format!(
        "{}▋{}",
        &app.input[..app.input_cursor],
        &app.input[app.input_cursor..]
    );

    let paragraph = Paragraph::new(display)
        .style(input_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(label)
                .border_style(Style::default().fg(
                    if app.is_thinking { Color::DarkGray } else { Color::Cyan }
                )),
        );

    frame.render_widget(paragraph, area);
}

// ── Fake AI response (Stage 3 placeholder) ───────────────────
// We'll replace this with a real OpenAI call in Stage 4.

async fn fake_ai_response(prompt: &str) -> String {
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    format!(
        "Echo: \"{}\"\n(OpenAI not wired up yet — see Stage 4!)",
        prompt
    )
}

// ── Main event loop ──────────────────────────────────────────
// This is the heart of any Ratatui app:
//
//   loop {
//     terminal.draw(|f| render(f, &state));   // 1. draw current state
//     let event = wait_for_event();            // 2. wait for input
//     update_state(&mut state, event);         // 3. update state
//   }

#[tokio::main]
async fn main() -> io::Result<()> {
    // ── Setup terminal ────────────────────────────────────────
    // Raw mode: keypresses go straight to us (no line buffering, no echo)
    // Alternate screen: like vim — restores the original terminal on exit
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    // ── Event loop ────────────────────────────────────────────
    loop {
        // Draw the current state
        terminal.draw(|f| draw(f, &app))?;

        // Poll for events (non-blocking, 50ms timeout so we can redraw)
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match (key.modifiers, key.code) {
                    // Quit
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) => break,

                    // Submit message
                    (_, KeyCode::Enter) => {
                        if let Some(prompt) = app.submit() {
                            app.is_thinking = true;
                            terminal.draw(|f| draw(f, &app))?; // show "thinking..."

                            // In Stage 4, swap fake_ai_response for real OpenAI call
                            let response = fake_ai_response(&prompt).await;

                            app.is_thinking = false;
                            app.messages.push(Message {
                                role: Role::Assistant,
                                content: response,
                            });
                        }
                    }

                    // Text input
                    (_, KeyCode::Char(c)) => app.insert_char(c),
                    (_, KeyCode::Backspace) => app.delete_char(),

                    // Cursor movement
                    (_, KeyCode::Left) => {
                        if app.input_cursor > 0 {
                            app.input_cursor -= 1;
                        }
                    }
                    (_, KeyCode::Right) => {
                        if app.input_cursor < app.input.len() {
                            app.input_cursor += 1;
                        }
                    }

                    // Scroll message history
                    (_, KeyCode::Up) => app.scroll_offset += 1,
                    (_, KeyCode::Down) => {
                        app.scroll_offset = app.scroll_offset.saturating_sub(1)
                    }

                    _ => {}
                }
            }
        }
    }

    // ── Teardown ──────────────────────────────────────────────
    // ALWAYS restore the terminal — if you skip this, the user's
    // terminal stays in raw mode after your app exits (very annoying)
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
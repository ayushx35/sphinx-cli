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
use tokio::sync::mpsc;
use futures_util::StreamExt;
use serde::Deserialize;

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: Delta,
}

#[derive(Deserialize)]
struct Delta {
    content: Option<String>,
}

enum AiEvent {
    Token(String),  // a new piece of text arrived
    Done,           // stream finished
    Error(String),  // something went wrong
}

// ── App State ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone)]
struct Message {
    role: Role,
    content: String,
}

struct App {
    messages: Vec<Message>,
    input: String,
    input_cursor: usize,
    scroll_offset: usize,
    is_streaming: bool,
    // The channel receiver lives here so the event loop can
    // poll it every frame with try_recv()
    ai_rx: Option<mpsc::UnboundedReceiver<AiEvent>>,
}

impl App {
    fn new() -> Self {
        Self {
            messages: vec![Message {
                role: Role::System,
                content: "OpenAI chat — type a message and press Enter. Ctrl+C to quit.".into(),
            }],
            input: String::new(),
            input_cursor: 0,
            scroll_offset: 0,
            is_streaming: false,
            ai_rx: None,
        }
    }

    fn insert_char(&mut self, c: char) {
        self.input.insert(self.input_cursor, c);
        self.input_cursor += c.len_utf8();
    }

    fn delete_char(&mut self) {
        if self.input_cursor > 0 {
            let prev = self.input[..self.input_cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.remove(prev);
            self.input_cursor = prev;
        }
    }

    fn submit(&mut self) -> Option<Vec<serde_json::Value>> {
        let text = self.input.trim().to_string();
        if text.is_empty() || self.is_streaming {
            return None;
        }

        self.messages.push(Message { role: Role::User, content: text.clone() });
        self.input.clear();
        self.input_cursor = 0;
        self.scroll_offset = 0;

    
        let history: Vec<serde_json::Value> = self.messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| serde_json::json!({
                "role": match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::System => "system",
                },
                "content": m.content
            }))
            .collect();

        Some(history)
    }

    // Called every frame to drain any tokens the background task sent.
    // Returns true if any tokens were received (so we know to redraw).
    fn poll_ai(&mut self) -> bool {
        let rx = match &mut self.ai_rx {
            Some(r) => r,
            None => return false,
        };

        let mut got_something = false;

        // try_recv is non-blocking — returns Err(Empty) immediately
        // if there's nothing waiting. Perfect for a render loop.
        loop {
            match rx.try_recv() {
                Ok(AiEvent::Token(token)) => {
                    got_something = true;
                    // Append token to the last assistant message,
                    // or create one if this is the first token
                    match self.messages.last_mut() {
                        Some(m) if m.role == Role::Assistant => {
                            m.content.push_str(&token);
                        }
                        _ => {
                            self.messages.push(Message {
                                role: Role::Assistant,
                                content: token,
                            });
                        }
                    }
                }
                Ok(AiEvent::Done) => {
                    self.is_streaming = false;
                    self.ai_rx = None;
                    got_something = true;
                    break;
                }
                Ok(AiEvent::Error(e)) => {
                    self.is_streaming = false;
                    self.ai_rx = None;
                    self.messages.push(Message {
                        role: Role::System,
                        content: format!("Error: {}", e),
                    });
                    got_something = true;
                    break;
                }
                Err(_) => break, // Empty or disconnected — stop draining
            }
        }

        got_something
    }
}

// ── OpenAI streaming task ─────────────────────────────────────
// This runs in its own tokio task, completely independent of the
// UI. It sends tokens through the channel as they arrive.
//
// HOW SSE WORKS:
//   1. We POST with "stream": true
//   2. The server keeps the connection open
//   3. It sends lines like: `data: {json}\n\n`
//   4. We read the byte stream, split on newlines, parse each chunk

async fn stream_openai(
    api_key: String,
    messages: Vec<serde_json::Value>,
    tx: mpsc::UnboundedSender<AiEvent>,
) {
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": "gpt-4o-mini",   // cheap and fast for practice
        "messages": messages,
        "stream": true,
        "max_tokens": 1024,
    });

    let response = match client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(AiEvent::Error(e.to_string()));
            return;
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let _ = tx.send(AiEvent::Error(format!("HTTP {}: {}", status, body)));
        return;
    }

    // response.bytes_stream() gives us a Stream<Item = Result<Bytes>>
    // We read it chunk by chunk. Each chunk may contain partial lines,
    // multiple lines, or a single line — so we buffer and split manually.
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        match chunk {
            Err(e) => {
                let _ = tx.send(AiEvent::Error(e.to_string()));
                return;
            }
            Ok(bytes) => {
                // Bytes aren't guaranteed to align to SSE boundaries,
                // so we accumulate into a buffer and process complete lines
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                // Process all complete lines in the buffer
                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim().to_string();
                    buffer = buffer[newline_pos + 1..].to_string();

                    // SSE lines look like:  data: {json}
                    // The stream ends with: data: [DONE]
                    if let Some(data) = line.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            let _ = tx.send(AiEvent::Done);
                            return;
                        }

                        // Parse the JSON chunk and extract the delta text
                        match serde_json::from_str::<StreamChunk>(data) {
                            Ok(chunk) => {
                                for choice in chunk.choices {
                                    if let Some(text) = choice.delta.content {
                                        if !text.is_empty() {
                                            let _ = tx.send(AiEvent::Token(text));
                                        }
                                    }
                                }
                            }
                            // Silently skip malformed chunks — the stream
                            // sometimes includes empty or comment lines
                            Err(_) => {}
                        }
                    }
                }
            }
        }
    }

    let _ = tx.send(AiEvent::Done);
}

// ── Drawing ──────────────────────────────────────────────────

fn draw(frame: &mut Frame, app: &App) {
    let size = frame.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(size);

    draw_messages(frame, app, chunks[0]);
    draw_input(frame, app, chunks[1]);
}

fn draw_messages(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    for msg in &app.messages {
        match msg.role {
            Role::User => {
                lines.push(Line::from(vec![
                    Span::styled("You  › ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw(msg.content.clone()),
                ]));
            }
            Role::Assistant => {
                // For streaming messages, add the blinking cursor at the end
                let content = if app.is_streaming {
                    format!("{}▋", msg.content)
                } else {
                    msg.content.clone()
                };
                lines.push(Line::from(vec![
                    Span::styled("AI   › ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw(content),
                ]));
            }
            Role::System => {
                lines.push(Line::from(Span::styled(
                    format!("     ℹ  {}", msg.content),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        lines.push(Line::from(""));
    }

    // Show "thinking..." only before the first token arrives
    if app.is_streaming && matches!(app.messages.last(), Some(m) if m.role != Role::Assistant) {
        lines.push(Line::from(Span::styled(
            "AI   › ▋",
            Style::default().fg(Color::Green).add_modifier(Modifier::DIM),
        )));
    }

    let visible_height = area.height.saturating_sub(2) as usize;
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll = (max_scroll.saturating_sub(app.scroll_offset)) as u16;

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" SPHINX-CLI "))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, area);
}

fn draw_input(frame: &mut Frame, app: &App, area: Rect) {
    let (label, border_color) = if app.is_streaming {
        (" streaming… ", Color::Yellow)
    } else {
        (" message ", Color::Cyan)
    };

    let display = format!(
        "{}▋{}",
        &app.input[..app.input_cursor],
        &app.input[app.input_cursor..]
    );

    let paragraph = Paragraph::new(display)
        .style(Style::default().fg(if app.is_streaming { Color::DarkGray } else { Color::White }))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(label)
                .border_style(Style::default().fg(border_color)),
        );

    frame.render_widget(paragraph, area);
}

// ── Main ─────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> io::Result<()> {
    // Read API key from environment — never hardcode secrets!
    let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| {
        eprintln!("Error: OPENAI_API_KEY environment variable not set.");
        eprintln!("Export it with: export OPENAI_API_KEY=sk-...");
        std::process::exit(1);
    });

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    loop {
        terminal.draw(|f| draw(f, &app))?;

        // ── Drain AI tokens first ─────────────────────────────
        // Check the channel before checking keyboard events.
        // This ensures tokens render even if the user isn't typing.
        app.poll_ai();

        // ── Handle keyboard events ────────────────────────────
        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                match (key.modifiers, key.code) {
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) => break,

                    (_, KeyCode::Enter) => {
                        if let Some(history) = app.submit() {
                            app.is_streaming = true;

                            // Create the channel.
                            // UnboundedSender can be cloned and sent across threads.
                            // UnboundedReceiver stays in App for polling.
                            let (tx, rx) = mpsc::unbounded_channel::<AiEvent>();
                            app.ai_rx = Some(rx);

                            let key_clone = api_key.clone();

                            // tokio::spawn launches an async task that runs
                            // concurrently with the event loop — no blocking!
                            tokio::spawn(async move {
                                stream_openai(key_clone, history, tx).await;
                            });
                        }
                    }

                    (_, KeyCode::Char(c)) => {
                        if !app.is_streaming {
                            app.insert_char(c);
                        }
                    }
                    (_, KeyCode::Backspace) => {
                        if !app.is_streaming {
                            app.delete_char();
                        }
                    }

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

                    (_, KeyCode::Up) => app.scroll_offset += 1,
                    (_, KeyCode::Down) => {
                        app.scroll_offset = app.scroll_offset.saturating_sub(1)
                    }

                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
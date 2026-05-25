# Sphinx-CLI 🤖💬

Sphinx-CLI is a lightweight, high-performance terminal chat interface for interacting with OpenAI's models in real-time. Built using **Rust**, **Ratatui**, and **Tokio**, it demonstrates how to handle asynchronous, non-blocking Server-Sent Events (SSE) streaming inside a terminal user interface (TUI) rendering loop.

---

## 📺 Demo

[Insert your product demo video/GIF here]

---

## ✨ Features

* **Real-Time Streaming:** Watch answers stream in token-by-token with a dynamic terminal blinking cursor.
* **Non-Blocking Async Architecture:** Powered by `tokio` and `futures-util`—the TUI stays interactive at 60 FPS while background HTTP network streams fetch chunks.
* **Intuitive TUI Layout:** Built using `ratatui` with responsive text-wrapping, distinct color coding for roles (System, User, Assistant), and a "thinking..." status.
* **History Scrolling:** Navigate long conversations effortlessly using your arrow keys (**Up/Down**).
* **Safe Secrets:** Relies on environment variables rather than hardcoded credentials.

---

## 🛠️ Tech Stack & Key Concepts

* **[Ratatui](https://github.com/ratatui-org/ratatui):** Handles terminal rendering, layout splits, constraints, and widget management.
* **[Tokio](https://tokio.rs/):** Provides the asynchronous runtime to handle concurrent event loops and spawn non-blocking network tasks.
* **MPSC Channels (`tokio::sync::mpsc`):** Bridges the gap between the async background worker (`stream_openai`) and the synchronous UI frame renderer via thread-safe, unbounded event queues (`AiEvent`).
* **Manual SSE Parsing:** Decodes incoming byte-streams into structured JSON (`serde_json`), accounting for partial HTTP chunks without breaking boundaries.

---

## 🚀 Getting Started

### Prerequisites

* Rust toolchain installed (MSRV 1.70.0+ recommended)
* An OpenAI API Key

### Installation & Setup

1. **Clone the repository:**
   ```bash
   git clone [https://github.com/yourusername/sphinx-cli.git](https://github.com/yourusername/sphinx-cli.git)
   cd sphinx-cl
   ```

2.   **Set up your Environment Variable:**
```bash
export OPENAI_API_KEY="sk-your-actual-api-key-here" 
```
3. **Run the application:**
```bash
cargo run --release
```
## 🎮 Controls

| Key | Action |
| :--- | :--- |
| `Enter` | Submit your prompt to the AI |
| `Up Arrow` | Scroll *up* through chat history |
| `Down Arrow`| Scroll *down* through chat history |
| `Left Arrow`| Move input cursor left |
| `Right Arrow`| Move input cursor right |
| `Backspace` | Delete character behind the cursor |
| `Ctrl + C` | Safely exit the application |

> ℹ️ **Note:** Input editing and backspaces are disabled while the AI is actively streaming a response to prevent concurrent mutations.
4. **Architecture Breakdown**
The program operates on a split-worker loop:
The Main Loop: Draws the frame at a smooth interval (≈16ms intervals / 60Hz), polls keyboard inputs using crossterm, and drains the ai_rx MPSC channel on every tick using try_recv().
The Worker Task: On pressing Enter, tokio::spawn ships the text history off into a background network routine. It handles the reqwest pipeline and continuously fires tokens back to the main thread until it hits [DONE] or encounters an error.
5. **License**
This project is licensed under the MIT License. See the LICENSE file for details.
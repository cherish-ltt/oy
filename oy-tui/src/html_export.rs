use crate::message::Message;
use oy_agent::oy_ai::Role;
use std::time::{SystemTime, UNIX_EPOCH};

/// Export messages to a self-contained HTML string.
pub fn export_session_to_html(messages: &[&Message]) -> String {
    let mut sidebar_items = Vec::new();
    let mut main_items = Vec::new();
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let timestamp = unix_to_readable(now_secs);

    for (i, msg) in messages.iter().enumerate() {
        match msg {
            Message::UiMessages(text) => {
                let snippet: String = text.chars().take(50).collect();
                sidebar_items.push(format!(
                    r#"<div class="sidebar-item" data-index="{}" data-role="system" onclick="scrollToMsg({})"><span class="role-badge role-system">System</span> {}</div>"#,
                    i, i, html_escape(&snippet)
                ));
                main_items.push(format!(
                    r#"<div id="msg-{}" class="message system-msg"><div class="msg-role">&gt; System Note</div><div class="msg-content">{}</div></div>"#,
                    i, html_escape(text)
                ));
            },
            Message::AgentMessages(chat_msg, _) => {
                let role_str = match chat_msg.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };
                let snippet: String = chat_msg
                    .content
                    .as_deref()
                    .unwrap_or("")
                    .split('\n')
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(50)
                    .collect();

                sidebar_items.push(format!(
                    r#"<div class="sidebar-item" data-index="{}" data-role="{}" onclick="scrollToMsg({})"><span class="role-badge role-{}">{}</span> {}</div>"#,
                    i, role_str, i, role_str, role_str, html_escape(&snippet)
                ));

                let mut body = String::new();

                // Thinking content (collapsible)
                if let Some(ref thinking) = chat_msg.reasoning_content {
                    body.push_str(&format!(
                        r#"<details class="thinking-block"><summary>&#x1f914; thinking</summary><pre>{}</pre></details>"#,
                        html_escape(thinking)
                    ));
                }

                // Main content
                if let Some(ref content) = chat_msg.content {
                    if role_str == "tool" {
                        body.push_str(&format!(
                            r#"<div class="tool-content"><pre>{}</pre></div>"#,
                            html_escape(content)
                        ));
                    } else {
                        body.push_str(&format!(
                            r#"<div class="msg-content">{}</div>"#,
                            html_escape(content)
                        ));
                    }
                }

                // Tool calls
                if let Some(ref tool_calls) = chat_msg.tool_calls {
                    for tc in tool_calls {
                        body.push_str(&format!(
                            r#"<div class="tool-call">&#x1f527; <strong>{}</strong>: <code>{}</code></div>"#,
                            html_escape(&tc.function_name),
                            html_escape(&tc.arguments.to_string())
                        ));
                    }
                }

                main_items.push(format!(
                    r#"<div id="msg-{}" class="message {}-msg"><div class="msg-role role-{}">{}</div>{}</div>"#,
                    i, role_str, role_str, role_str, body
                ));
            },
            Message::ToolCallMsg(state) => {
                let snippet = &state.function_name;
                sidebar_items.push(format!(
                    r#"<div class="sidebar-item" data-index="{}" data-role="tool" onclick="scrollToMsg({})"><span class="role-badge role-tool">Tool</span> &#x1f527; {}</div>"#,
                    i, i, html_escape(snippet)
                ));

                let duration = state
                    .end_time
                    .map(|e| {
                        let d = e.saturating_duration_since(state.start_time);
                        d.as_secs_f64()
                    })
                    .unwrap_or_else(|| state.start_time.elapsed().as_secs_f64());
                let icon = if state.result.is_some() {
                    "&#x2713;"
                } else {
                    "&middot;"
                };
                let mut body = format!(
                    r#"<div class="tool-call-header">&#x1f527; ToolCall {} <strong>{}</strong>: {} ({:.1}s/{}s)</div>"#,
                    icon,
                    html_escape(&state.function_name),
                    html_escape(state.arguments.as_deref().unwrap_or("unknown arguments")),
                    duration,
                    state.timeout_secs,
                );
                if let Some(ref result) = state.result
                    && let Some(ref content) = result.content
                {
                    body.push_str(&format!(
                        r#"<div class="tool-result"><pre>{}</pre></div>"#,
                        html_escape(content)
                    ));
                }
                main_items.push(format!(
                    r#"<div id="msg-{}" class="message tool-call-msg">{}</div>"#,
                    i, body
                ));
            },
            // Skip AgentStatus and PromptQueued
            _ => {},
        }
    }

    TEMPLATE_HTML
        .replace("{sidebar_items}", &sidebar_items.join("\n"))
        .replace("{main_items}", &main_items.join("\n"))
        .replace("{timestamp}", &timestamp)
}

/// Convert Unix timestamp seconds to a human-readable "YYYY-MM-DD HH:MM:SS" UTC string.
fn unix_to_readable(secs: u64) -> String {
    let days = secs / 86400;
    let remaining = secs % 86400;
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;

    let mut y: i64 = 1970;
    let mut d = days;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if d < days_in_year {
            break;
        }
        d -= days_in_year;
        y += 1;
    }

    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 1usize;
    for &md in month_days.iter() {
        if d < md {
            break;
        }
        d -= md;
        m += 1;
    }
    let day = d + 1;

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y, m, day, hours, minutes, seconds
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Escape HTML special characters to prevent XSS.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const TEMPLATE_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>OY Session Export</title>
<style>
  *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
  :root {
    --bg: #ffffff;
    --fg: #1a1a1a;
    --sidebar-bg: #f5f5f5;
    --sidebar-fg: #333;
    --border: #ddd;
    --system-bg: #f0f0f0;
    --system-fg: #333;
    --user-bg: #e3f2fd;
    --user-fg: #1565c0;
    --assistant-bg: #e8f5e9;
    --assistant-fg: #2e7d32;
    --tool-bg: #f3e5f5;
    --tool-fg: #7b1fa2;
    --thinking-bg: #f1f8e9;
    --thinking-fg: #558b2f;
    --accent: #1976d2;
    --badge-system: #9e9e9e;
    --badge-user: #42a5f5;
    --badge-assistant: #66bb6a;
    --badge-tool: #ab47bc;
    --msg-max-width: 800px;
  }
  body.dark {
    --bg: #1e1e1e;
    --fg: #e0e0e0;
    --sidebar-bg: #252525;
    --sidebar-fg: #ccc;
    --border: #444;
    --system-bg: #2d2d2d;
    --system-fg: #ccc;
    --user-bg: #1a237e;
    --user-fg: #90caf9;
    --assistant-bg: #1b5e20;
    --assistant-fg: #a5d6a7;
    --tool-bg: #4a148c;
    --tool-fg: #ce93d8;
    --thinking-bg: #33691e;
    --thinking-fg: #c5e1a5;
    --accent: #64b5f6;
    --badge-system: #757575;
    --badge-user: #1e88e5;
    --badge-assistant: #43a047;
    --badge-tool: #8e24aa;
  }
  html, body { height: 100%; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; background: var(--bg); color: var(--fg); }
  #app { display: flex; height: 100vh; }
  #sidebar { width: 300px; min-width: 300px; background: var(--sidebar-bg); border-right: 1px solid var(--border); display: flex; flex-direction: column; overflow: hidden; }
  .sidebar-header { display: flex; align-items: center; justify-content: space-between; padding: 12px 16px; border-bottom: 1px solid var(--border); }
  .sidebar-header h2 { font-size: 16px; font-weight: 700; }
  #theme-toggle { background: none; border: 1px solid var(--border); border-radius: 6px; padding: 4px 10px; cursor: pointer; font-size: 16px; color: var(--fg); }
  #theme-toggle:hover { background: var(--border); }
  #search { margin: 8px 12px; padding: 8px 12px; border: 1px solid var(--border); border-radius: 6px; font-size: 13px; background: var(--bg); color: var(--fg); outline: none; }
  #search:focus { border-color: var(--accent); }
  .role-filters { display: flex; flex-wrap: wrap; gap: 4px; padding: 0 12px 8px; }
  .filter-btn { padding: 4px 10px; border: 1px solid var(--border); border-radius: 12px; background: var(--bg); color: var(--fg); font-size: 12px; cursor: pointer; transition: all 0.15s; }
  .filter-btn:hover { border-color: var(--accent); }
  .filter-btn.active { background: var(--accent); color: #fff; border-color: var(--accent); }
  #message-list { flex: 1; overflow-y: auto; padding: 4px 0; }
  .sidebar-item { padding: 8px 16px; cursor: pointer; border-bottom: 1px solid var(--border); font-size: 13px; display: flex; align-items: center; gap: 6px; transition: background 0.12s; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sidebar-item:hover { background: var(--border); }
  .sidebar-item.hidden { display: none; }
  .role-badge { display: inline-block; padding: 1px 8px; border-radius: 10px; font-size: 11px; font-weight: 600; color: #fff; flex-shrink: 0; }
  .role-badge.role-system { background: var(--badge-system); }
  .role-badge.role-user { background: var(--badge-user); }
  .role-badge.role-assistant { background: var(--badge-assistant); }
  .role-badge.role-tool { background: var(--badge-tool); }
  #main-content { flex: 1; overflow-y: auto; padding: 24px; }
  .message { max-width: var(--msg-max-width); margin-bottom: 16px; padding: 12px 16px; border-radius: 8px; line-height: 1.55; font-size: 14px; }
  .system-msg { background: var(--system-bg); color: var(--system-fg); }
  .user-msg { background: var(--user-bg); color: var(--user-fg); }
  .assistant-msg { background: var(--assistant-bg); color: var(--assistant-fg); }
  .tool-msg { background: var(--tool-bg); color: var(--tool-fg); }
  .tool-call-msg { background: var(--tool-bg); color: var(--tool-fg); }
  .msg-role { font-size: 12px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 6px; opacity: 0.8; }
  .msg-role.role-system { color: var(--system-fg); }
  .msg-role.role-user { color: var(--user-fg); }
  .msg-role.role-assistant { color: var(--assistant-fg); }
  .msg-role.role-tool { color: var(--tool-fg); }
  .msg-content { white-space: pre-wrap; word-break: break-word; }
  .msg-content p { margin-bottom: 8px; }
  .thinking-block { margin: 6px 0; background: var(--thinking-bg); border-radius: 6px; padding: 8px 12px; }
  .thinking-block summary { cursor: pointer; font-style: italic; color: var(--thinking-fg); font-size: 13px; font-weight: 600; }
  .thinking-block pre { margin-top: 6px; white-space: pre-wrap; font-size: 13px; color: var(--thinking-fg); }
  .tool-content { margin-top: 6px; }
  .tool-content pre { white-space: pre-wrap; font-size: 13px; }
  .tool-call { margin-top: 4px; font-size: 13px; }
  .tool-call code { background: rgba(0,0,0,0.08); padding: 1px 5px; border-radius: 3px; font-size: 12px; }
  body.dark .tool-call code { background: rgba(255,255,255,0.1); }
  .tool-call-header { font-weight: 600; margin-bottom: 4px; font-size: 13px; }
  .tool-result { margin-top: 4px; }
  .tool-result pre { white-space: pre-wrap; font-size: 13px; }
  .footer { text-align: center; font-size: 12px; color: var(--border); padding: 16px; border-top: 1px solid var(--border); margin-top: 24px; }
  @media (max-width: 768px) {
    #sidebar { width: 200px; min-width: 200px; }
    #main-content { padding: 16px; }
  }
</style>
</head>
<body>
<div id="app" class="theme-light">
  <aside id="sidebar">
    <div class="sidebar-header">
      <h2>Messages</h2>
      <button id="theme-toggle" title="Toggle dark/light mode">&#x1f313;</button>
    </div>
    <input type="text" id="search" placeholder="Search messages..." />
    <div class="role-filters">
      <button class="filter-btn active" data-role="all">All</button>
      <button class="filter-btn" data-role="system">System</button>
      <button class="filter-btn" data-role="user">User</button>
      <button class="filter-btn" data-role="assistant">AI</button>
      <button class="filter-btn" data-role="tool">Tool</button>
    </div>
    <div id="message-list">
{sidebar_items}
    </div>
  </aside>
  <main id="main-content">
{main_items}
    <div class="footer">Exported at {timestamp}</div>
  </main>
</div>
<script>
  (function() {
    // ── Theme ──
    var saved = localStorage.getItem('oy-theme') || 'light';
    var app = document.getElementById('app');
    app.className = 'theme-' + saved;
    if (saved === 'dark') document.body.classList.add('dark');
    document.getElementById('theme-toggle').addEventListener('click', function() {
      var isDark = app.className === 'theme-dark';
      app.className = isDark ? 'theme-light' : 'theme-dark';
      document.body.classList.toggle('dark', !isDark);
      localStorage.setItem('oy-theme', isDark ? 'light' : 'dark');
    });

    // ── Role filtering ──
    var filterBtns = document.querySelectorAll('.filter-btn');
    var sidebarItems = document.querySelectorAll('.sidebar-item');
    var mainMessages = document.querySelectorAll('.message');
    filterBtns.forEach(function(btn) {
      btn.addEventListener('click', function() {
        filterBtns.forEach(function(b) { b.classList.remove('active'); });
        btn.classList.add('active');
        var role = btn.getAttribute('data-role');
        sidebarItems.forEach(function(item) {
          if (role === 'all' || item.getAttribute('data-role') === role) {
            item.classList.remove('hidden');
          } else {
            item.classList.add('hidden');
          }
        });
        mainMessages.forEach(function(msg) {
          if (role === 'all') {
            msg.style.display = '';
          } else {
            var match = msg.classList.contains(role + '-msg') || msg.classList.contains('tool-call-msg');
            msg.style.display = match ? '' : 'none';
          }
        });
      });
    });

    // ── Search ──
    document.getElementById('search').addEventListener('input', function() {
      var q = this.value.toLowerCase();
      sidebarItems.forEach(function(item) {
        var text = item.textContent.toLowerCase();
        item.classList.toggle('hidden', text.indexOf(q) === -1);
      });
    });

    // ── Scroll to message ──
    window.scrollToMsg = function(index) {
      var el = document.getElementById('msg-' + index);
      if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' });
      // Also activate the corresponding sidebar item (brief highlight)
      var items = document.querySelectorAll('.sidebar-item');
      items.forEach(function(item) { item.style.background = ''; });
      var target = document.querySelector('.sidebar-item[data-index="' + index + '"]');
      if (target) {
        target.style.background = 'var(--border)';
        setTimeout(function() { target.style.background = ''; }, 600);
      }
    };
  })();
</script>
</body>
</html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ToolCallState;
    use oy_agent::oy_ai::ChatMessage;
    use std::time::Instant;

    #[test]
    fn test_export_empty_messages() {
        let html = export_session_to_html(&[]);
        assert!(html.contains("<html"));
        assert!(html.contains("</html>"));
    }

    #[test]
    fn test_export_ui_message() {
        let msg = Message::UiMessages("Hello world".into());
        let html = export_session_to_html(&[&msg]);
        assert!(html.contains("Hello world"));
    }

    #[test]
    fn test_export_user_message() {
        let chat = ChatMessage::user("Hello from user");
        let msg = Message::AgentMessages(chat, false);
        let html = export_session_to_html(&[&msg]);
        assert!(html.contains("Hello from user"));
        assert!(html.contains("user"));
    }

    #[test]
    fn test_export_assistant_with_thinking() {
        let chat = ChatMessage::assistant(
            Some("Final answer".into()),
            Some("I need to think...".into()),
            None,
        );
        let msg = Message::AgentMessages(chat, false);
        let html = export_session_to_html(&[&msg]);
        assert!(html.contains("Final answer"));
        assert!(html.contains("I need to think..."));
        assert!(html.contains("details"));
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("\"hello\""), "&quot;hello&quot;");
        assert_eq!(html_escape("foo & bar"), "foo &amp; bar");
    }

    #[test]
    fn test_sidebar_has_role_badges() {
        let msgs: Vec<Message> = vec![
            Message::AgentMessages(ChatMessage::user("user msg"), false),
            Message::AgentMessages(
                ChatMessage::assistant(Some("assistant msg".into()), None, None),
                false,
            ),
        ];
        let refs: Vec<&Message> = msgs.iter().collect();
        let html = export_session_to_html(&refs);
        assert!(html.contains("role-user"));
        assert!(html.contains("role-assistant"));
    }

    #[test]
    fn test_dark_light_toggle_button() {
        let html = export_session_to_html(&[]);
        assert!(html.contains("theme-toggle"));
    }

    #[test]
    fn test_search_input_exists() {
        let html = export_session_to_html(&[]);
        assert!(html.contains("id=\"search\""));
    }

    #[test]
    fn test_role_filter_buttons() {
        let html = export_session_to_html(&[]);
        assert!(html.contains("data-role=\"all\""));
        assert!(html.contains("data-role=\"system\""));
        assert!(html.contains("data-role=\"user\""));
        assert!(html.contains("data-role=\"assistant\""));
        assert!(html.contains("data-role=\"tool\""));
    }

    #[test]
    fn test_export_agent_status_skipped() {
        let msg = Message::AgentStatus(crate::message::Status::Pause);
        let html = export_session_to_html(&[&msg]);
        // AgentStatus should not appear in output
        assert!(!html.contains("AgentStatus"));
        assert!(!html.contains("pause"));
    }

    #[test]
    fn test_export_prompt_queued_skipped() {
        let msg = Message::PromptQueued {
            id: uuid::Uuid::now_v7(),
            text: "test prompt".into(),
        };
        let html = export_session_to_html(&[&msg]);
        // PromptQueued should not appear in output
        assert!(!html.contains("PromptQueued"));
    }

    #[test]
    fn test_tool_call_msg_export() {
        let state = ToolCallState {
            function_name: "Bash".into(),
            arguments: Some("ls -la".into()),
            tool_call_id: "call_1".into(),
            result: Some(ChatMessage::tool(
                "file list output",
                "call_1".into(),
                None,
                None,
            )),
            start_time: Instant::now(),
            end_time: Some(Instant::now()),
            expanded: false,
            timeout_secs: 150,
        };
        let msg = Message::ToolCallMsg(state);
        let html = export_session_to_html(&[&msg]);
        assert!(html.contains("Bash"));
        assert!(html.contains("ls -la"));
        assert!(html.contains("file list output"));
    }

    #[test]
    fn test_unix_to_readable() {
        // 2026-06-14 09:50:00 UTC approximate
        let result = unix_to_readable(1780465800);
        // Just verify it returns something in expected format
        assert!(result.len() == 19); // "YYYY-MM-DD HH:MM:SS"
        assert!(result.contains('-'));
        assert!(result.contains(':'));
    }
}

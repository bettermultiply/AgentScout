//! Terminal presentation. It only reads the run view and submits UI requests.
use super::{
    ui::{Interface, Request},
    view,
};
use ascout::control_plane::agent_control::BranchRole;
use ascout::control_plane::gateway::ui::Connection;
use ascout::control_plane::ledger::LedgerReader;
use std::io::{self, Read, Write};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

struct Terminal(libc::termios);
impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &self.0);
        }
        print!("\x1b[?25h\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}

pub fn start(
    interface: Interface,
    connection: Connection,
    reader: LedgerReader,
) -> io::Result<JoinHandle<()>> {
    let mut original = std::mem::MaybeUninit::uninit();
    if unsafe { libc::tcgetattr(0, original.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let original = unsafe { original.assume_init() };
    let mut raw = original;
    unsafe {
        libc::cfmakeraw(&mut raw);
    }
    if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } != 0 {
        return Err(io::Error::last_os_error());
    }
    print!("\x1b[?1049h\x1b[?25l");
    io::stdout().flush()?;
    Ok(std::thread::spawn(move || {
        let _terminal = Terminal(original);
        if let Err(error) = run(&interface, &connection, &reader) {
            eprintln!("TUI: {error}");
            let _ = connection.request(Request::Shutdown);
        }
    }))
}

fn run(interface: &Interface, connection: &Connection, reader: &LedgerReader) -> io::Result<()> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for byte in io::stdin().lock().bytes() {
            if tx.send(byte).is_err() {
                break;
            }
        }
    });
    run_input(interface, connection, reader, rx)
}

fn run_input(
    interface: &Interface,
    connection: &Connection,
    reader: &LedgerReader,
    rx: mpsc::Receiver<io::Result<u8>>,
) -> io::Result<()> {
    let (mut input, mut escape) = (Vec::new(), Vec::new());
    let (mut observing, mut events, mut facts) = (false, false, false);
    let mut selected: Option<String> = None;
    let (mut scroll, mut previous) = (0usize, String::new());
    let mut hint = String::new();
    let mut prompt_id = None;
    let mut prompt_error = String::new();
    let mut answer = Vec::new();
    let mut choice = 0usize;
    loop {
        let state = super::ui::run_view(reader.state_event_set());
        if !state.connected {
            break;
        }
        let prompt = state
            .prompts
            .iter()
            .find(|(_, (owner, _))| Some(owner) == state.tree.authority_branch_id.as_ref());
        let next_id = prompt.map(|(id, _)| id.clone());
        if prompt_id != next_id {
            prompt_id = next_id;
            prompt_error.clear();
            answer = prompt
                .and_then(|(_, (_, p))| p.prefill.as_ref())
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_default();
            choice = 0;
            escape.clear();
            scroll = 0;
        }
        let id = selected
            .as_ref()
            .or(state.tree.authority_branch_id.as_ref());
        let mut content = String::new();
        if events {
            for event in &state.events {
                content.push_str(&format!("{}\n", serde_json::to_string(event)?));
            }
        } else if observing && selected.is_none() {
            content.push_str("Tree — 只读观察\n");
            for (index, branch) in state.tree.branches.values().enumerate() {
                content.push_str(&format!(
                    "\n{}  {}  parent={}  {} / {}{}\n",
                    index + 1,
                    branch.branch_id,
                    branch.parent_branch_id.as_deref().unwrap_or("root"),
                    view::role_label(branch.role),
                    view::status_label(branch.status),
                    if Some(&branch.branch_id) == state.tree.authority_branch_id.as_ref() {
                        " [当前主对话]"
                    } else {
                        ""
                    }
                ));
            }
            content.push_str("\n输入编号；Esc 返回主对话。查看不代表接受。\n");
        } else if let Some(id) = id
            && let Some(branch) = state.tree.branches.get(id)
        {
            if observing {
                content.push_str(&view::branch_details(
                    branch,
                    &state.tree,
                    "运行事实见 Tab 事件视图；候选身份见角色与状态",
                ));
            }
            if facts {
                for event in state.events.iter().filter(|e| e["branch_id"] == *id) {
                    content.push_str(&view::structured(event));
                }
            } else if let Some(conversation) = interface.conversation(id) {
                if conversation.role == BranchRole::Candidate {
                    content.push_str("\n候选内容；查看不代表接受。\n");
                }
                if !conversation.inherited.is_empty() {
                    content.push_str("\n── 继承上下文 ──\n");
                }
                for message in &conversation.inherited {
                    content.push_str(&message_text(message));
                }
                content.push_str("\n━━ 本 Branch 独有内容 ━━\n");
                for message in &conversation.messages {
                    content.push_str(&message_text(message));
                }
                content.push_str(&conversation.streaming);
            }
        }
        if let Some((_, (_, prompt))) = prompt {
            content = format!(
                "{}\n{}\n",
                prompt.title,
                prompt.message_text.as_deref().unwrap_or("")
            );
            if prompt.kind == "confirm" || prompt.kind == "select" {
                let options = if prompt.kind == "confirm" {
                    vec!["是".into(), "否".into()]
                } else {
                    prompt.options.clone()
                };
                for (index, option) in options.iter().enumerate() {
                    content.push_str(&format!(
                        "{} {}\n",
                        if index == choice { "❯" } else { " " },
                        option
                    ));
                }
            } else {
                content.push_str(prompt.placeholder.as_deref().unwrap_or(""));
                content.push('\n');
                content.push_str(&String::from_utf8_lossy(&answer));
            }
            if !prompt_error.is_empty() {
                content.push_str(&format!("\n{prompt_error}\n"));
            }
        } else if !observing && !events && input.starts_with(b"/") {
            let prefix = String::from_utf8_lossy(&input);
            content.push_str("\n命令（Tab 补全第一项，Enter 执行）\n");
            for command in &state.commands {
                let name = format!("/{}", command["name"].as_str().unwrap_or(""));
                if name.starts_with(prefix.as_ref()) {
                    content.push_str(&format!(
                        "{}  {}\n",
                        name,
                        command["description"].as_str().unwrap_or("")
                    ));
                }
            }
        }
        draw(
            &content,
            &String::from_utf8_lossy(if prompt.is_some() { &answer } else { &input }),
            if let Some((_, (_, p))) = prompt {
                if p.kind == "editor" {
                    "Enter 换行；Ctrl-S 提交；Esc 取消；Ctrl-C 中断；Ctrl-D 退出"
                } else {
                    "↑/↓ 选择；Enter 提交；Esc 取消；Ctrl-C 中断；Ctrl-D 退出"
                }
            } else if observing || events {
                &hint
            } else {
                &state.notice
            },
            scroll,
            &mut previous,
        )?;
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(byte)) => {
                if !escape.is_empty() {
                    escape.push(byte);
                    if escape == b"\x1b[A" {
                        if prompt.is_some() {
                            choice = choice.saturating_sub(1);
                        } else {
                            scroll = scroll.saturating_add(3);
                        }
                        escape.clear();
                    } else if escape == b"\x1b[B" {
                        if let Some((_, (_, p))) = prompt {
                            choice = (choice + 1).min(if p.kind == "confirm" {
                                1
                            } else {
                                p.options.len().saturating_sub(1)
                            });
                        } else {
                            scroll = scroll.saturating_sub(3);
                        }
                        escape.clear();
                    } else if escape.len() > 3 || (escape.len() == 2 && byte != b'[') {
                        escape.clear();
                    }
                    continue;
                }
                if let Some((id, (_, p))) = prompt {
                    match byte {
                        4 => {
                            let _ = connection.request(Request::Shutdown);
                        }
                        3 => {
                            let _ = connection.request(Request::Interrupt);
                        }
                        27 => {
                            escape.push(27);
                        }
                        127 | 8 => {
                            answer.pop();
                            while !answer.is_empty() && std::str::from_utf8(&answer).is_err() {
                                answer.pop();
                            }
                        }
                        b'\r' | b'\n' if p.kind == "editor" => answer.push(b'\n'),
                        b'\r' | b'\n' | 19 => {
                            let value = match p.kind.as_str() {
                                "confirm" => serde_json::json!(choice == 0),
                                "select" => p
                                    .options
                                    .get(choice)
                                    .map(|s| serde_json::json!(s))
                                    .unwrap_or(serde_json::Value::Null),
                                _ => serde_json::json!(String::from_utf8_lossy(&answer)),
                            };
                            if let Err(error) = connection.request(Request::UiPromptAnswer {
                                prompt_id: id.clone(),
                                answer: value,
                                cancelled: false,
                            }) {
                                prompt_error = error;
                            }
                        }
                        byte if byte >= 32 && (p.kind == "input" || p.kind == "editor") => {
                            answer.push(byte)
                        }
                        _ => {}
                    }
                    continue;
                }
                match byte {
                    4 => {
                        let _ = connection.request(Request::Shutdown);
                    }
                    3 => {
                        input.clear();
                        let _ = connection.request(Request::Interrupt);
                    }
                    27 => {
                        if observing || events {
                            observing = false;
                            events = false;
                            selected = None;
                            facts = false;
                            input.clear();
                            scroll = 0;
                        } else {
                            escape.push(27);
                        }
                    }
                    b'\t' if selected.is_some() => {
                        facts = !facts;
                        scroll = 0;
                    }
                    b'\t' if !observing && !events && input.starts_with(b"/") => {
                        let prefix = String::from_utf8_lossy(&input);
                        if let Some(name) = state
                            .commands
                            .iter()
                            .filter_map(|c| c["name"].as_str())
                            .map(|n| format!("/{n}"))
                            .find(|n| n.starts_with(prefix.as_ref()))
                        {
                            input = name.into_bytes();
                        }
                    }
                    127 | 8 => {
                        input.pop();
                        while !input.is_empty() && std::str::from_utf8(&input).is_err() {
                            input.pop();
                        }
                    }
                    b'\r' | b'\n' => {
                        let line = String::from_utf8_lossy(&input).to_string();
                        input.clear();
                        scroll = 0;
                        if line == "/pause" {
                            let _ = connection.request(Request::Pause);
                        } else if line == "/continue" {
                            let _ = connection.request(Request::Continue);
                        } else if line == "/ascout" {
                            observing = true;
                            events = false;
                            selected = None;
                            hint = "输入编号；Tab 切换运行事实；Esc 返回".into();
                        } else if line == "/events" {
                            events = true;
                            observing = false;
                            hint = "事件（只读）；Esc 返回".into();
                        } else if observing {
                            match view::select_branch(&state.tree, &line) {
                                Ok(id) => {
                                    selected = Some(id);
                                    facts = false;
                                    hint = "只读观察；Tab 切换运行事实；Esc 返回".into();
                                }
                                Err(error) => hint = error,
                            }
                        } else if !events && !line.trim().is_empty() {
                            let _ = connection.request(Request::Prompt { prompt: line });
                        }
                    }
                    byte if byte >= 32 => input.push(byte),
                    _ => {}
                }
            }
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = connection.request(Request::Shutdown);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if escape == b"\x1b"
                    && let Some((id, _)) = prompt
                {
                    if let Err(error) = connection.request(Request::UiPromptAnswer {
                        prompt_id: id.clone(),
                        answer: serde_json::Value::Null,
                        cancelled: true,
                    }) {
                        prompt_error = error;
                    }
                }
                escape.clear();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ascout::control_plane::agent_control::UiPrompt;
    use ascout::control_plane::ledger::Ledger;
    use ascout::control_plane::observation::ObservationContent;
    use std::sync::{Arc, Mutex};

    #[test]
    fn terminal_keys_return_typed_answers_and_cancel_without_chat() {
        for (kind, keys, expected, cancelled) in [
            (
                "confirm",
                b"\x1b[B\r".as_slice(),
                serde_json::json!(false),
                false,
            ),
            (
                "select",
                b"\x1b[B\r".as_slice(),
                serde_json::json!("b"),
                false,
            ),
            (
                "input",
                "你好\x7f界\r".as_bytes(),
                serde_json::json!("你界"),
                false,
            ),
            (
                "editor",
                b"\nsecond\x13".as_slice(),
                serde_json::json!("first\nsecond"),
                false,
            ),
            ("input", b"\x1b".as_slice(), serde_json::Value::Null, true),
        ] {
            let path = std::path::PathBuf::from(format!(
                "/tmp/opencode/tui-prompt-{}-{kind}-{cancelled}.jsonl",
                std::process::id()
            ));
            let mut ledger = Ledger::open(&path).unwrap();
            ledger
                .append_observation(
                    "task",
                    ObservationContent::AuthorityChanged {
                        branch_id: "authority".into(),
                        previous_authority: None,
                    },
                )
                .unwrap();
            ledger
                .append_observation(
                    "task",
                    ObservationContent::UiPromptRequested {
                        branch_id: "authority".into(),
                        prompt_id: "prompt".into(),
                        prompt: UiPrompt {
                            kind: kind.into(),
                            title: "Question".into(),
                            message_text: None,
                            options: vec!["a".into(), "b".into()],
                            placeholder: None,
                            prefill: (kind == "editor").then(|| "first".into()),
                        },
                    },
                )
                .unwrap();
            let reader = ledger.reader();
            let ledger = Arc::new(Mutex::new(ledger));
            let replies = Arc::new(Mutex::new(Vec::new()));
            let captured = replies.clone();
            let connection = Connection::new(reader.clone(), move |request| {
                let Request::UiPromptAnswer {
                    prompt_id,
                    answer,
                    cancelled,
                } = request
                else {
                    panic!("prompt keys became a chat or lifecycle request")
                };
                captured.lock().unwrap().push((answer, cancelled));
                let mut ledger = ledger.lock().unwrap();
                ledger
                    .append_observation(
                        "task",
                        ObservationContent::UiPromptResolved {
                            branch_id: "authority".into(),
                            prompt_id,
                            cancelled,
                            reason: None,
                        },
                    )
                    .unwrap();
                ledger
                    .append_observation(
                        "task",
                        ObservationContent::RunStatus {
                            ready: false,
                            running: false,
                            connected: false,
                            notice: String::new(),
                        },
                    )
                    .unwrap();
                Ok(())
            });
            let interface = Interface::new("task".into(), reader.clone());
            let (tx, rx) = mpsc::channel();
            for byte in keys {
                tx.send(Ok(*byte)).unwrap();
            }
            // Keep the sender alive so a lone Escape is recognized on timeout.
            run_input(&interface, &connection, &reader, rx).unwrap();
            assert_eq!(*replies.lock().unwrap(), vec![(expected, cancelled)]);
            drop(tx);
            std::fs::remove_file(path).unwrap();
        }
    }
}

fn message_text(entry: &serde_json::Value) -> String {
    let mut text = format!("\n{}\n", entry["role"].as_str().unwrap_or("message"));
    if let Some(value) = entry["content"].as_str() {
        text.push_str(value);
    } else if let Some(parts) = entry["content"].as_array() {
        for part in parts {
            match part["type"].as_str() {
                Some("text") => text.push_str(part["text"].as_str().unwrap_or("")),
                Some("thinking") => text.push_str(part["thinking"].as_str().unwrap_or("")),
                Some("toolCall") => text.push_str(&format!(
                    "\n工具 {}\n{}\n",
                    part["name"].as_str().unwrap_or(""),
                    view::structured(&part["arguments"])
                )),
                _ => text.push_str("[暂未展示的内容]"),
            }
        }
    }
    if let Some(error) = entry["errorMessage"].as_str() {
        text.push_str(error);
    }
    text.push('\n');
    text
}

fn draw(
    content: &str,
    input: &str,
    notice: &str,
    scroll: usize,
    previous: &mut String,
) -> io::Result<()> {
    let mut size = std::mem::MaybeUninit::<libc::winsize>::zeroed();
    unsafe {
        libc::ioctl(1, libc::TIOCGWINSZ, size.as_mut_ptr());
    }
    let size = unsafe { size.assume_init() };
    let width = usize::from(size.ws_col).max(20) - 1;
    let height = usize::from(size.ws_row).max(8) - 4;
    let clean = |s: &str| {
        s.chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect::<String>()
    };
    let mut lines = Vec::new();
    for line in clean(content).lines() {
        let chars: Vec<_> = line.chars().collect();
        if chars.is_empty() {
            lines.push(String::new())
        } else {
            for chunk in chars.chunks(width) {
                lines.push(chunk.iter().collect::<String>())
            }
        }
    }
    let end = lines
        .len()
        .saturating_sub(scroll.min(lines.len().saturating_sub(height)));
    let mut frame =
        String::from("\x1b[H\x1b[2JAScout · /ascout: Tree · /events: Events · Ctrl-D: exit\r\n");
    for index in 0..height {
        if let Some(line) = lines
            .get(end.saturating_sub(height) + index)
            .filter(|_| end.saturating_sub(height) + index < end)
        {
            frame.push_str(line)
        }
        frame.push_str("\r\n");
    }
    frame.push_str(
        &clean(notice)
            .replace('\n', " ")
            .chars()
            .take(width)
            .collect::<String>(),
    );
    frame.push_str("\r\n❯ ");
    frame.push_str(
        &clean(input)
            .replace('\n', " ↵ ")
            .chars()
            .rev()
            .take(width.saturating_sub(2))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>(),
    );
    if frame != *previous {
        io::stdout().write_all(frame.as_bytes())?;
        io::stdout().flush()?;
        *previous = frame;
    }
    Ok(())
}

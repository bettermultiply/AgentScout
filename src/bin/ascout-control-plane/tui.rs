//! Terminal presentation. It only reads the run view and submits UI requests.
use super::{
    ui::{Interface, Request},
    view,
};
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

pub fn start(interface: Interface) -> io::Result<JoinHandle<()>> {
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
        if let Err(error) = run(&interface) {
            eprintln!("TUI: {error}");
            let _ = interface.request(Request::Shutdown);
        }
    }))
}

fn run(interface: &Interface) -> io::Result<()> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for byte in io::stdin().lock().bytes() {
            if tx.send(byte).is_err() {
                break;
            }
        }
    });
    let (mut input, mut escape) = (Vec::new(), Vec::new());
    let (mut observing, mut events, mut facts) = (false, false, false);
    let mut selected: Option<String> = None;
    let (mut scroll, mut previous) = (0usize, String::new());
    let mut hint = String::new();
    loop {
        let state = interface.run_view();
        if !state.connected {
            break;
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
        } else if let Some(id) = id {
            if let Some(branch) = state.tree.branches.get(id) {
                if observing {
                    content.push_str(&view::branch_details(
                        branch,
                        &state.tree,
                        state
                            .facts
                            .get(id)
                            .map(String::as_str)
                            .unwrap_or("尚无运行事实"),
                    ));
                }
                if facts {
                    for event in state.events.iter().filter(|e| e["branch_id"] == *id) {
                        content.push_str(&view::structured(event));
                    }
                } else if let Some(conversation) = state.conversations.get(id) {
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
        }
        draw(
            &content,
            &String::from_utf8_lossy(&input),
            if observing || events {
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
                        scroll = scroll.saturating_add(3);
                        escape.clear();
                    } else if escape == b"\x1b[B" {
                        scroll = scroll.saturating_sub(3);
                        escape.clear();
                    } else if escape.len() > 3 || (escape.len() == 2 && byte != b'[') {
                        escape.clear();
                    }
                    continue;
                }
                match byte {
                    4 => {
                        let _ = interface.request(Request::Shutdown);
                    }
                    3 => {
                        input.clear();
                        let _ = interface.request(Request::Interrupt);
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
                    127 | 8 => {
                        input.pop();
                        while !input.is_empty() && std::str::from_utf8(&input).is_err() {
                            input.pop();
                        }
                    }
                    b'\r' | b'\n' => {
                        let line = String::from_utf8_lossy(&input).trim().to_string();
                        input.clear();
                        scroll = 0;
                        if line == "/ascout" {
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
                        } else if !events && !line.is_empty() {
                            let _ = interface.request(Request::Prompt { prompt: line });
                        }
                    }
                    byte if byte >= 32 => input.push(byte),
                    _ => {}
                }
            }
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = interface.request(Request::Shutdown);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => escape.clear(),
        }
    }
    Ok(())
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

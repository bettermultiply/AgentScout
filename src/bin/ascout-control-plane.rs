//! AScout 自有终端界面。Agent 产品只通过接入协议提供输出，不提供界面。

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ascout::agent_control::{AgentCommand, AgentController, AgentMessage, AgentWireMessage};
use ascout::transport;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() != 2 || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(
            "usage (in a terminal): ascout-control-plane <workspace-root> <pi-root>".into(),
        );
    }
    let workspace = fs::canonicalize(&args[0])?;
    let pi_root = fs::canonicalize(&args[1])?;
    // 每次运行单独创建管理目录，绝不删除用户提供的 workspace。
    let managed = tempfile::Builder::new()
        .prefix("ascout-")
        .tempdir_in(&workspace)?;
    let socket_dir = tempfile::Builder::new()
        .prefix("ascout-")
        .tempdir_in("/tmp/opencode")?;
    let socket = socket_dir.path().join("agent.sock");
    let listener = UnixListener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    let log = fs::File::create(managed.path().join("agent.log"))?;
    let mut agent = Command::new("node")
        .arg("--import")
        .arg(pi_root.join("packages/coding-agent/src/experimental/source-resolver.ts"))
        .arg(pi_root.join("packages/coding-agent/src/core/agent-access.ts"))
        .arg(&socket)
        .arg("agent")
        .current_dir(pi_root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if let Some(status) = agent.try_wait()? {
                        return Err(format!(
                            "Agent exited: {status}; see {}/agent.log",
                            managed.path().display()
                        )
                        .into());
                    }
                    if Instant::now() >= deadline {
                        return Err("Agent connection timed out".into());
                    }
                    thread::sleep(Duration::from_millis(30));
                }
                Err(error) => return Err(error.into()),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        match transport::read_message(&mut stream)? {
            Some(AgentWireMessage::Message(AgentMessage::Hello { agent_id }))
                if agent_id == "agent" => {}
            _ => return Err("Agent handshake failed".into()),
        }
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut controller = AgentController::new("agent", managed.path())?;
        // 第一阶段仅在本次受管目录运行，不把用户目录冒充可清理环境。
        let command = controller.create_authority("authority", "")?;
        send(&mut stream, command)?;
        let mut terminal = std::mem::MaybeUninit::<libc::termios>::uninit();
        if unsafe { libc::tcgetattr(0, terminal.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let terminal = unsafe { terminal.assume_init() };
        let mut raw = terminal;
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        print!("\x1b[?1049h\x1b[?25l");
        io::stdout().flush()?;
        let result = run(&mut stream, &mut controller);
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &terminal) };
        print!("\x1b[?25h\x1b[?1049l");
        io::stdout().flush()?;
        result
    })();
    let _ = agent.kill();
    let _ = agent.wait();
    // 保留会话诊断与运行目录供用户查看；不声称异常退出已完成环境清理。
    let retained = managed.keep();
    eprintln!("AScout run directory: {}", retained.display());
    result
}

fn run(
    stream: &mut UnixStream,
    controller: &mut AgentController,
) -> Result<(), Box<dyn std::error::Error>> {
    let (tx, rx) = mpsc::channel();
    let input_tx = tx.clone();
    thread::spawn(move || {
        for byte in io::stdin().lock().bytes() {
            match byte {
                Ok(byte) => {
                    if input_tx.send(Ok(byte)).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let mut reader = stream.try_clone()?;
    thread::spawn(move || {
        loop {
            let message = transport::read_message::<_, AgentWireMessage>(&mut reader);
            let done = !matches!(&message, Ok(Some(_)));
            if tx.send(Err(message)).is_err() || done {
                break;
            }
        }
    });
    let mut chat = String::from(
        "AScout — connected to Pi\nCommands: /ascout to observe the Tree. Ctrl-C: interrupt. Ctrl-D: exit.\nSpeculation is not connected yet; no candidates are fabricated.\n",
    );
    let mut details: BTreeMap<String, String> = BTreeMap::new();
    let mut input = Vec::new();
    let mut observing = false;
    let mut selected: Option<String> = None;
    let mut ready = false;
    let mut running = false;
    let mut notice = String::from("Starting Agent session…");
    let mut scroll = 0usize;
    let mut escape = Vec::new();
    loop {
        let content = if observing {
            if let Some(id) = &selected {
                format!(
                    "Branch {id} — read-only; observation does not accept a candidate\n{}\n{}",
                    serde_json::to_string_pretty(&controller.state().tree.branches[id])?,
                    details
                        .get(id)
                        .map(String::as_str)
                        .unwrap_or("No output reported.")
                )
            } else {
                let tree = &controller.state().tree;
                let mut text = format!(
                    "Tree: {} — read-only\nCurrent authority: {}\n",
                    tree.agent_id,
                    tree.authority_branch_id.as_deref().unwrap_or("none")
                );
                for (index, branch) in tree.branches.values().enumerate() {
                    text.push_str(&format!(
                        "\n{}  {}  parent={}  {:?} / {:?}{}\n",
                        index + 1,
                        branch.branch_id,
                        branch.parent_branch_id.as_deref().unwrap_or("root"),
                        branch.role,
                        branch.status,
                        if tree.authority_branch_id.as_deref() == Some(&branch.branch_id) {
                            " [current main conversation]"
                        } else {
                            " [NOT VALIDATED]"
                        }
                    ));
                }
                text.push_str("\nEnter a number to inspect a Branch. No candidates means no speculation has run.\n");
                text
            }
        } else {
            chat.clone()
        };
        render(
            &content,
            &String::from_utf8_lossy(&input),
            &notice,
            observing,
            scroll,
        )?;
        match rx.recv_timeout(Duration::from_millis(150)) {
            Ok(Ok(byte)) => {
                if !escape.is_empty() {
                    escape.push(byte);
                    if escape.len() == 2 && byte != b'[' {
                        escape.clear();
                    } else if escape.len() >= 3 && byte.is_ascii_alphabetic() || escape.len() > 8 {
                        if escape == b"\x1b[A" {
                            scroll = scroll.saturating_add(3);
                        }
                        if escape == b"\x1b[B" {
                            scroll = scroll.saturating_sub(3);
                        }
                        escape.clear();
                    }
                    continue;
                }
                match byte {
                    4 => break,
                    3 => {
                        input.clear();
                        if let Some(id) = &controller.state().tree.authority_branch_id {
                            send(
                                stream,
                                AgentCommand::Abort {
                                    branch_id: id.clone(),
                                },
                            )?;
                            notice =
                                "Interrupt requested; waiting for Agent execution to settle".into();
                        }
                    }
                    27 => {
                        if observing {
                            observing = false;
                            selected = None;
                            input.clear();
                            scroll = 0;
                        } else {
                            escape.push(27);
                        }
                    }
                    127 | 8 => {
                        input.pop();
                        while std::str::from_utf8(&input).is_err() {
                            input.pop();
                        }
                    }
                    b'\r' | b'\n' => {
                        let line = String::from_utf8_lossy(&input).trim().to_string();
                        input.clear();
                        scroll = 0;
                        if line == "/ascout" {
                            observing = !observing;
                            selected = None;
                        } else if observing {
                            selected = line
                                .parse::<usize>()
                                .ok()
                                .and_then(|n| n.checked_sub(1))
                                .and_then(|n| {
                                    controller.state().tree.branches.keys().nth(n).cloned()
                                });
                        } else if !line.is_empty() {
                            if !ready || running {
                                input = line.into_bytes();
                                notice = "Wait for the current response, or press Ctrl-C to interrupt. Input retained.".into();
                            } else if let Some(id) = &controller.state().tree.authority_branch_id {
                                chat.push_str(&format!("\nYou: {line}\n\nAgent: "));
                                send(
                                    stream,
                                    AgentCommand::Prompt {
                                        branch_id: id.clone(),
                                        prompt: line,
                                    },
                                )?;
                                running = true;
                                notice = "Agent running".into();
                            }
                        }
                    }
                    byte if byte >= 32 => input.push(byte),
                    _ => {}
                }
            }
            Ok(Err(Ok(Some(AgentWireMessage::Message(message))))) => {
                match &message {
                    AgentMessage::Output { branch_id, text } => {
                        details.entry(branch_id.clone()).or_default().push_str(text);
                        if controller.state().tree.authority_branch_id.as_ref() == Some(branch_id) {
                            chat.push_str(text);
                        }
                    }
                    AgentMessage::BranchStarted { branch_id } => {
                        details
                            .entry(branch_id.clone())
                            .or_default()
                            .push_str("\n[Agent session actually started]\n");
                        if controller.state().tree.authority_branch_id.as_ref() == Some(branch_id) {
                            ready = true;
                            notice = "Ready | /ascout: Tree | Esc: conversation | ↑/↓: scroll | Ctrl-D: exit".into();
                        }
                    }
                    AgentMessage::TurnCompleted { branch_id } => {
                        details
                            .entry(branch_id.clone())
                            .or_default()
                            .push_str("\n[Execution finished; not an acceptance decision]\n");
                        if controller.state().tree.authority_branch_id.as_ref() == Some(branch_id) {
                            running = false;
                            chat.push('\n');
                            notice =
                                "Ready | /ascout: Tree | Ctrl-C: interrupt | Ctrl-D: exit".into();
                        }
                    }
                    AgentMessage::BranchFailed { branch_id, reason } => {
                        details
                            .entry(branch_id.clone())
                            .or_default()
                            .push_str(&format!("\n[Failed] {reason}\n"));
                        notice = format!("Branch {branch_id} failed: {reason}");
                        ready = false;
                        running = false;
                    }
                    AgentMessage::ParentOutcome { .. }
                    | AgentMessage::CandidateCompleted { .. } => {
                        return Err(
                            "Unexpected speculative result: pending-result path is not connected"
                                .into(),
                        );
                    }
                    _ => {}
                }
                for command in controller.handle_message(message)? {
                    send(stream, command)?;
                }
            }
            Ok(Err(Ok(Some(AgentWireMessage::Command(_))))) => {
                return Err("Agent sent a control-plane command".into());
            }
            Ok(Err(Ok(None))) => return Err("Agent disconnected; cleanup is not confirmed".into()),
            Ok(Err(Err(error))) => return Err(error.into()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                escape.clear();
            }
            Err(_) => break,
        }
    }
    for branch in controller.state().tree.branches.values() {
        send(
            stream,
            AgentCommand::Abort {
                branch_id: branch.branch_id.clone(),
            },
        )?;
    }
    Ok(())
}

fn render(
    content: &str,
    input: &str,
    notice: &str,
    observing: bool,
    scroll: usize,
) -> io::Result<()> {
    let mut size = std::mem::MaybeUninit::<libc::winsize>::zeroed();
    unsafe { libc::ioctl(1, libc::TIOCGWINSZ, size.as_mut_ptr()) };
    let size = unsafe { size.assume_init() };
    let width = usize::from(size.ws_col).max(20).saturating_sub(1);
    let height = usize::from(size.ws_row).max(8).saturating_sub(4);
    // 不执行 Agent 输出中的终端控制序列，避免候选输出改变界面或伪造状态。
    let clean: String = content
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    let mut lines = Vec::new();
    for line in clean.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            lines.push(String::new());
        } else {
            for chunk in chars.chunks(width) {
                lines.push(chunk.iter().collect::<String>());
            }
        }
    }
    let end = lines
        .len()
        .saturating_sub(scroll.min(lines.len().saturating_sub(height)));
    let start = end.saturating_sub(height);
    let mut frame = String::from("\x1b[H\x1b[2JAScout | ");
    frame.push_str(if observing {
        "Tree observation (read-only)"
    } else {
        "Main conversation"
    });
    frame.push_str("\r\n");
    for row in 0..height {
        if let Some(line) = lines.get(start + row).filter(|_| start + row < end) {
            frame.push_str(line);
        }
        frame.push_str("\r\n");
    }
    frame.push_str(
        &notice
            .chars()
            .filter(|c| !c.is_control())
            .take(width)
            .collect::<String>(),
    );
    frame.push_str("\r\n> ");
    let prompt: String = input.chars().filter(|c| !c.is_control()).collect();
    let skip = prompt
        .chars()
        .count()
        .saturating_sub(width.saturating_sub(2));
    frame.extend(prompt.chars().skip(skip));
    let mut out = io::stdout().lock();
    out.write_all(frame.as_bytes())?;
    out.flush()
}

fn send(stream: &mut UnixStream, command: AgentCommand) -> io::Result<()> {
    transport::write_message(stream, &AgentWireMessage::Command(command))
}

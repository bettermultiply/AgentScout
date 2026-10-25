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
use ascout::policy::{CompiledPolicy, Decision, PolicyEngine};
use ascout::transport;
use serde_json::Value;
use serde_json::json;

struct Pending {
    parent: String,
    operation_id: String,
    candidate: String,
    proposal: Value,
    outcome: Option<Value>,
    started: Instant,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if !(2..=3).contains(&args.len()) || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(
            "usage (in a terminal): ascout-control-plane <workspace-root> <pi-root> [policy.yaml]"
                .into(),
        );
    }
    let workspace = fs::canonicalize(&args[0])?;
    let pi_root = fs::canonicalize(&args[1])?;
    let policy = PolicyEngine::default().compile(std::path::Path::new(
        args.get(2)
            .map(String::as_str)
            .unwrap_or("assets/policy/policy.yaml"),
    ))?;
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
        let result = run(&mut stream, &mut controller, &policy);
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
    policy: &CompiledPolicy,
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
        "AScout — connected to Pi\nCommands: /ascout to observe the Tree. Ctrl-C: interrupt. Ctrl-D: exit.\nReal speculation: waiting for a supported tool boundary.\n",
    );
    let mut details: BTreeMap<String, String> = BTreeMap::new();
    let mut pending: BTreeMap<String, Pending> = BTreeMap::new();
    let mut buffered: BTreeMap<String, String> = BTreeMap::new();
    let mut finished = std::collections::BTreeSet::new();
    let mut sequence = 0;
    let mut events = fs::File::create(controller.state().workspace_root.join("events.jsonl"))?;
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
                            " [not current authority]"
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
                        for (_, work) in std::mem::take(&mut pending) {
                            send(
                                stream,
                                AgentCommand::ResolvePending {
                                    branch_id: work.parent,
                                    operation_id: work.operation_id,
                                    accepted: false,
                                },
                            )?;
                            for command in controller.resolve_candidate(&work.candidate, false)? {
                                send(stream, command)?;
                            }
                        }
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
                                finished.remove(id);
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
                writeln!(events, "{}", serde_json::to_string(&message)?)?;
                match &message {
                    AgentMessage::CleanupCompleted { branch_id } => {
                        details.entry(branch_id.clone()).or_default().push_str("\n[CleanupCompleted: Agent session stopped/released; managed directory cleanup follows]\n");
                    }
                    AgentMessage::Output { branch_id, text } => {
                        details.entry(branch_id.clone()).or_default().push_str(text);
                        if controller.state().tree.authority_branch_id.as_ref() == Some(branch_id) {
                            chat.push_str(text);
                        } else if pending.values().any(|work| &work.candidate == branch_id) {
                            buffered
                                .entry(branch_id.clone())
                                .or_default()
                                .push_str(text);
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
                        finished.insert(branch_id.clone());
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
                    AgentMessage::ToolPending {
                        branch_id,
                        operation_id,
                        tool_name,
                        arguments,
                        context,
                    } => {
                        let predictions = if controller.state().tree.authority_branch_id.as_ref()
                            == Some(branch_id)
                        {
                            match predict(policy, operation_id, tool_name, arguments) {
                                Ok(value) => value,
                                Err(error) => {
                                    details.entry(branch_id.clone()).or_default().push_str(
                                        &format!("\n[Policy error; Parent fallback] {error}\n"),
                                    );
                                    Vec::new()
                                }
                            }
                        } else {
                            Vec::new()
                        };
                        if predictions.is_empty() {
                            notice = format!(
                                "ToolPending {tool_name}; Policy did not admit speculation; Parent continues"
                            );
                        }
                        for prediction in predictions {
                            sequence += 1;
                            let candidate_id = format!("candidate-{sequence}");
                            let _ = controller.create_candidate(&branch_id, &candidate_id, "")?;
                            let workspace = controller.state().tree.branches[&candidate_id]
                                .workspace
                                .clone();
                            let source = &controller.state().tree.branches[branch_id].workspace;
                            // Copy only ordinary files/directories; never follow links into other workspaces.
                            if let Err(error) = copy_workspace(source, &workspace) {
                                details
                                    .entry(candidate_id.clone())
                                    .or_default()
                                    .push_str(&format!("[Workspace copy failed] {error}\n"));
                                for command in
                                    controller.handle_message(AgentMessage::BranchFailed {
                                        branch_id: candidate_id,
                                        reason: error.to_string(),
                                    })?
                                {
                                    send(stream, command)?;
                                }
                                continue;
                            }
                            let pending_id = format!("{operation_id}:{candidate_id}");
                            pending.insert(
                                pending_id.clone(),
                                Pending {
                                    parent: branch_id.clone(),
                                    operation_id: operation_id.clone(),
                                    candidate: candidate_id.clone(),
                                    proposal: prediction.clone(),
                                    outcome: None,
                                    started: Instant::now(),
                                },
                            );
                            send(
                                stream,
                                AgentCommand::StartCandidate {
                                    branch_id: candidate_id.clone(),
                                    parent_branch_id: branch_id.clone(),
                                    operation_id: operation_id.clone(),
                                    tool_name: tool_name.clone(),
                                    workspace,
                                    prediction,
                                    context: context.clone(),
                                },
                            )?;
                            details.entry(candidate_id).or_default().push_str(&format!(
                                "[Candidate requested for {operation_id}; hypothesis={}]\n",
                                pending[&pending_id].proposal
                            ));
                            notice = format!(
                                "ToolPending {tool_name}; {} Candidates started; Parent continues",
                                pending
                                    .values()
                                    .filter(|work| work.parent == *branch_id)
                                    .count()
                            );
                        }
                    }
                    AgentMessage::ParentOutcome {
                        branch_id,
                        operation_id,
                        outcome,
                    } => {
                        for work in pending.values_mut().filter(|work| {
                            work.parent == *branch_id && work.operation_id == *operation_id
                        }) {
                            work.outcome = Some(outcome.clone());
                            details
                                .entry(work.candidate.clone())
                                .or_default()
                                .push_str(&format!("\n[Actual Parent outcome] {outcome}\n"));
                        }
                        continue;
                    }
                    AgentMessage::BranchFailed { branch_id, reason } => {
                        if controller
                            .state()
                            .tree
                            .branches
                            .get(branch_id)
                            .is_some_and(|branch| {
                                matches!(
                                    branch.status,
                                    ascout::agent_control::BranchStatus::Discarded
                                        | ascout::agent_control::BranchStatus::Cleaned
                                )
                            })
                        {
                            continue;
                        }
                        let keys: Vec<_> = pending
                            .iter()
                            .filter(|(_, work)| {
                                &work.candidate == branch_id || &work.parent == branch_id
                            })
                            .map(|(id, _)| id.clone())
                            .collect();
                        for key in keys {
                            let work = pending.remove(&key).unwrap();
                            send(
                                stream,
                                AgentCommand::ResolvePending {
                                    branch_id: work.parent.clone(),
                                    operation_id: work.operation_id.clone(),
                                    accepted: false,
                                },
                            )?;
                            if &work.candidate != branch_id {
                                for command in
                                    controller.resolve_candidate(&work.candidate, false)?
                                {
                                    send(stream, command)?;
                                }
                            }
                        }
                        details
                            .entry(branch_id.clone())
                            .or_default()
                            .push_str(&format!("\n[Failed] {reason}\n"));
                        notice = format!("Branch {branch_id} failed: {reason}");
                        if controller.state().tree.authority_branch_id.as_ref() == Some(branch_id) {
                            ready = false;
                            running = false;
                        }
                    }
                    AgentMessage::CandidateCompleted { .. } => {
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
        let keys: Vec<_> = pending
            .iter()
            .filter(|(_, work)| {
                (work.outcome.is_some()
                    && controller.state().tree.branches[&work.candidate].status
                        == ascout::agent_control::BranchStatus::Running)
                    || work.started.elapsed() > Duration::from_secs(120)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for key in keys {
            let Some(work) = pending.remove(&key) else {
                continue;
            };
            let accepted = work
                .outcome
                .as_ref()
                .map(|outcome| matches_outcome(policy, &work.operation_id, &work.proposal, outcome))
                .transpose()
                .unwrap_or_else(|error| {
                    details
                        .entry(work.candidate.clone())
                        .or_default()
                        .push_str(&format!("\n[Policy error; Parent fallback] {error}\n"));
                    None
                })
                .unwrap_or(false)
                && controller.state().tree.authority_branch_id.as_ref() == Some(&work.parent)
                && controller.state().tree.branches[&work.candidate].status
                    == ascout::agent_control::BranchStatus::Running;
            send(
                stream,
                AgentCommand::ResolvePending {
                    branch_id: work.parent.clone(),
                    operation_id: work.operation_id.clone(),
                    accepted,
                },
            )?;
            // Created sessions that failed to start are cancelled, not accepted.
            let commands = if controller.state().tree.branches[&work.candidate].status
                == ascout::agent_control::BranchStatus::Running
            {
                controller.resolve_candidate(&work.candidate, accepted)?
            } else {
                controller.handle_message(AgentMessage::BranchFailed {
                    branch_id: work.candidate.clone(),
                    reason: "Candidate startup timed out".into(),
                })?
            };
            for command in commands {
                send(stream, command)?;
            }
            details
                .entry(work.candidate.clone())
                .or_default()
                .push_str(if accepted {
                    "\n[Policy disposition: Promote]\n"
                } else {
                    "\n[Policy disposition/fallback: Discard]\n"
                });
            writeln!(
                events,
                "{}",
                json!({"branch_id": work.candidate, "disposition": if accepted { "Promote" } else { "Discard" }, "hypothesis": work.proposal, "actual": work.outcome})
            )?;
            if accepted {
                // Promote invalidates every other continuation of the old Parent,
                // including candidates for other concurrently pending operations.
                let obsolete: Vec<_> = pending
                    .iter()
                    .filter(|(_, sibling)| sibling.parent == work.parent)
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in obsolete {
                    let sibling = pending.remove(&id).unwrap();
                    for command in controller.resolve_candidate(&sibling.candidate, false)? {
                        send(stream, command)?;
                    }
                    buffered.remove(&sibling.candidate);
                    details
                        .entry(sibling.candidate.clone())
                        .or_default()
                        .push_str("\n[Discard: another Candidate replaced Parent]\n");
                    writeln!(
                        events,
                        "{}",
                        json!({"branch_id": sibling.candidate, "operation_id": sibling.operation_id, "disposition": "Discard", "reason": "Parent replaced"})
                    )?;
                }
                chat.push_str(&buffered.remove(&work.candidate).unwrap_or_default());
                running = !finished.contains(&work.candidate);
                ready = true;
            } else {
                buffered.remove(&work.candidate);
            }
            notice = if accepted {
                "Candidate promoted; same live session now serves the main conversation".into()
            } else {
                "Candidate discarded; Parent continues with the actual tool result".into()
            };
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

fn predict(
    policy: &CompiledPolicy,
    operation_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
    let engine = PolicyEngine::default();
    let op = json!({"kind": tool_name, "produces_result": true, "result_pending": true, "has_predictor": true});
    let opportunity = engine.evaluate(policy, Decision::OpportunityPass, &json!({"op": op}))?;
    let feasibility = engine.evaluate(policy, Decision::FeasibilityPass, &json!({"op": op, "state": {}, "contract_context": {"arguments": arguments, "isolation": "directory-only", "candidate_tools": "reads_and_literal_delay_printf"}}))?;
    if opportunity != true || feasibility != true {
        return Ok(Vec::new());
    }
    let edge = json!({"edge_id": operation_id, "tool_name": tool_name, "arguments": arguments});
    let selected = engine.evaluate(
        policy,
        Decision::Select,
        &json!({"frontier_view": [edge], "scheduling_context": {"capacity": 1}}),
    )?;
    if !selected
        .as_array()
        .is_some_and(|ids| ids.contains(&json!(operation_id)))
    {
        return Ok(Vec::new());
    }
    let data = json!({
        "edge_context": edge,
        "process_context": {"operation_id": operation_id},
        "assumption_set": [],
        "capability_context": {"tool": tool_name, "text_result_only": true}
    });
    let result = engine.evaluate(policy, Decision::Predict, &data)?;
    if engine.evaluate(
        policy,
        Decision::Launch,
        &json!({"edge_context": edge, "authority_state": {}, "candidate_view": []}),
    )? != true
    {
        return Ok(Vec::new());
    }
    Ok(result["proposals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("payload").cloned())
        .collect())
}

fn matches_outcome(
    policy: &CompiledPolicy,
    operation_id: &str,
    proposal: &serde_json::Value,
    outcome: &serde_json::Value,
) -> Result<bool, Box<dyn std::error::Error>> {
    let result = PolicyEngine::default().evaluate(
        policy,
        Decision::Match,
        &json!({
            "proposal": {"payload": proposal},
            "evidence": outcome,
            "match_context": {"operation_id": operation_id}
        }),
    )?;
    let compatible = result["verdict"] == "compatible";
    let disposition = PolicyEngine::default().evaluate(
        policy,
        Decision::Dispose,
        &json!({
            "branch": {"operation_id": operation_id},
            "resolution": if compatible { "accepted" } else { "rejected" },
            "context": {"match": result}
        }),
    )?;
    Ok(compatible && disposition["actions"][0]["intent"] == "Promote")
}

fn copy_workspace(source: &std::path::Path, target: &std::path::Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            fs::create_dir(&destination)?;
            copy_workspace(&entry.path(), &destination)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), destination)?;
        } else {
            return Err(io::Error::other(
                "workspace contains a symlink or special file; speculation refused",
            ));
        }
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

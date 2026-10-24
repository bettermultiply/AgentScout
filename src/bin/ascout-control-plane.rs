//! 真实 Pi Agent 的最小 AScout 控制面入口。
//!
//! 该进程拥有 AgentController 和 Tree；Pi 只通过 Unix socket 接收命令、运行
//! AgentSession 并报告事实。这个入口用于外部端到端测试，不是固定场景驱动器。

use std::env;
use std::fs;
use std::io;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use ascout::agent_control::{AgentCommand, AgentController, AgentMessage, AgentWireMessage};
use ascout::policy::{CompiledPolicy, Decision, PolicyEngine};
use ascout::transport;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() != 4 {
        return Err(
            "usage: ascout-control-plane <socket> <workspace-root> <policy-artifact> <prompt>"
                .into(),
        );
    }
    let socket = PathBuf::from(&args[0]);
    let workspace = PathBuf::from(&args[1]);
    let artifact = PathBuf::from(&args[2]);
    let prompt = args[3].clone();
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    let (mut stream, _) = listener.accept()?;
    let mut controller = AgentController::new("pi-agent", &workspace)?;
    let policy = PolicyEngine::default();
    let compiled: CompiledPolicy = serde_json::from_slice(&fs::read(artifact)?)?;
    let mut proposal_payload = String::new();

    send(
        &mut stream,
        AgentWireMessage::Message(AgentMessage::Hello {
            agent_id: "pi-agent".into(),
        }),
    )?;
    let authority = controller.create_authority("authority", &prompt)?;
    send(&mut stream, AgentWireMessage::Command(authority))?;

    let mut candidate_started = false;
    while let Some(message) = transport::read_message::<_, AgentWireMessage>(&mut stream)? {
        match message {
            AgentWireMessage::Message(message) => {
                match message {
                    AgentMessage::Hello { .. } => {}
                    AgentMessage::BranchStarted { branch_id } => {
                        controller.handle_message(AgentMessage::BranchStarted {
                            branch_id: branch_id.clone(),
                        })?;
                        if branch_id == "authority" && !candidate_started {
                            let edge_id = "candidate";
                            let op = json!({
                                "kind": "agent_prompt",
                                "produces_result": true,
                                "result_pending": true,
                                "has_predictor": true
                            });
                            let admitted = policy.evaluate(&compiled, Decision::OpportunityPass, &json!({"op": op}))? == json!(true)
                            && policy.evaluate(&compiled, Decision::FeasibilityPass, &json!({"op": op, "state": {}, "contract_context": {"tools": []}}))? == json!(true);
                            let selected = admitted && policy.evaluate(&compiled, Decision::Select, &json!({
                            "frontier_view": [{"edge_id": edge_id}], "scheduling_context": {}
                        }))? == json!([edge_id]);
                            if selected && policy.evaluate(&compiled, Decision::Launch, &json!({
                            "edge_context": {"edge_id": edge_id}, "authority_state": {"started": true}, "candidate_view": []
                        }))? == json!(true) {
                            let prediction = policy.evaluate(&compiled, Decision::Predict, &json!({
                                "edge_context": {"edge_id": edge_id},
                                "process_context": {},
                                "assumption_set": [],
                                "capability_context": {"tools": []}
                            }))?;
                            proposal_payload = prediction["proposals"][0]["payload"]
                                .as_str()
                                .ok_or("Policy prediction did not contain text payload")?
                                .to_owned();
                            let candidate_prompt = format!(
                                "{prompt}\nThis is a speculative candidate. Respond with exactly: {proposal_payload}"
                            );
                            let candidate = controller.create_candidate("authority", edge_id, candidate_prompt)?;
                            send(&mut stream, AgentWireMessage::Command(candidate))?;
                            candidate_started = true;
                        } else {
                            eprintln!("{{\"event\":\"ordinary\",\"reason\":\"policy\"}}");
                        }
                        }
                    }
                    AgentMessage::CandidateOutcome {
                        branch_id, output, ..
                    } => {
                        let evidence = json!({"payload": output});
                        let proposal = json!({"payload": proposal_payload});
                        let matched = policy.evaluate(
                            &compiled,
                            Decision::Match,
                            &json!({
                                "proposal": proposal, "evidence": evidence, "match_context": {}
                            }),
                        )?;
                        let compatible = matched["verdict"] == "compatible";
                        let resolution = if compatible { "accepted" } else { "rejected" };
                        let disposition = policy.evaluate(&compiled, Decision::Dispose, &json!({
                            "branch": {"branch_id": branch_id}, "resolution": resolution, "context": {}
                        }))?;
                        eprintln!(
                            "{{\"event\":\"policy\",\"match\":{},\"disposition\":{}}}",
                            matched, disposition
                        );
                        let promote = compatible
                            && disposition == json!({"actions": [{"intent": "Promote"}]});
                        let commands = controller.resolve_candidate(&branch_id, promote)?;
                        for command in commands {
                            if let AgentCommand::Cleanup { branch_id, .. } = &command {
                                send(&mut stream, AgentWireMessage::Command(command.clone()))?;
                                controller.cleanup_completed(branch_id)?;
                            } else {
                                send(&mut stream, AgentWireMessage::Command(command))?;
                            }
                        }
                        println!("{}", serde_json::to_string_pretty(controller.state())?);
                        break;
                    }
                    AgentMessage::BranchFailed { branch_id, reason } => {
                        let commands = controller
                            .handle_message(AgentMessage::BranchFailed { branch_id, reason })?;
                        for command in commands {
                            if let AgentCommand::Cleanup { branch_id, .. } = &command {
                                send(&mut stream, AgentWireMessage::Command(command.clone()))?;
                                controller.cleanup_completed(branch_id)?;
                            } else {
                                send(&mut stream, AgentWireMessage::Command(command))?;
                            }
                        }
                        break;
                    }
                }
            }
            AgentWireMessage::Command(_) => {
                return Err("Pi sent a command to the control plane".into());
            }
        }
    }
    let _ = fs::remove_file(socket);
    Ok(())
}

fn send(stream: &mut std::os::unix::net::UnixStream, message: AgentWireMessage) -> io::Result<()> {
    transport::write_message(stream, &message)
}

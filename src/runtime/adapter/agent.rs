//! Adapter 管理的 Agent 子进程与状态监视。

use super::*;
use crate::protocol::contract::*;
use std::fs::OpenOptions;
use std::io;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;


impl Shared {
    /// 启动 Agent 接入侧。控制面决定用哪个产品，Runtime 负责启动。
    /// 一个 run 只有一个接入侧进程，它服务所有环境。
    pub(super) fn launch_agent(&self, agent: &AgentLaunch) -> io::Result<()> {
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&agent.log)?;
        let mut command = Command::new(&agent.command);
        command
            .args(&agent.args)
            .current_dir(&agent.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .envs(&agent.env);
        let mut slot = self.agent.lock().unwrap();
        if slot.is_some() {
            return Ok(());
        }
        *slot = Some(command.spawn()?);
        Ok(())
    }

    /// 停止 Agent 接入侧。它属于整个 run，不属于单个环境。
    pub(super) fn stop_agent(&self) {
        if let Some(mut child) = self.agent.lock().unwrap().take() {
            eprintln!("adapter: stopping agent pid {}", child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub(super) fn stop_agents(&self) {
        if self.runtime.manages_agent_process() {
            if let Err(error) = self.runtime.stop_agents() {
                eprintln!("adapter: failed to stop Runtime Agent: {error}");
            }
        } else {
            self.stop_agent();
        }
    }

    /// Adapter 退出路径：先停止 Agent，再让 Runtime 回收所有环境。
    pub(super) fn shutdown(&self) {
        self.stop_agents();
        if self.preserving.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        if let Err(error) = self.runtime.shutdown() {
            eprintln!("adapter: Runtime shutdown incomplete: {error}");
        }
    }
    pub(super) fn start_state_watcher(self: Arc<Self>) {
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(100));
                for environment_id in self.runtime.environments() {
                    let Some(current) = self.runtime.state(&environment_id) else {
                        continue;
                    };
                    let previous = self.known.lock().unwrap().get(&environment_id).copied();
                    match previous {
                        None => {
                            self.known.lock().unwrap().insert(environment_id, current);
                        }
                        Some(previous) if previous == current => {}
                        Some(previous) => {
                            self.known
                                .lock()
                                .unwrap()
                                .insert(environment_id.clone(), current);
                            let reason = if current == EnvironmentState::Terminated {
                                "environment stopped unexpectedly"
                            } else {
                                "environment state changed"
                            };
                            let event = Event {
                                environment_id,
                                request_id: None,
                                payload: EventPayload::EnvironmentStateChanged {
                                    previous: Some(previous),
                                    current,
                                    reason: reason.to_string(),
                                },
                            };
                            let _ = self.out.send(AdapterMessage::Event(event));
                        }
                    }
                }
            }
        });
    }}

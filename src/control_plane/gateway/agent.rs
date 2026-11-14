//! Agent listener, handshake, typed ingress and command delivery.
use crate::control_plane::agent_control::{
    AdapterCapabilities, AgentCommand, AgentMessage, AgentWireMessage,
};
use crate::protocol::transport;
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

pub struct Listener {
    listener: UnixListener,
}

impl Listener {
    pub fn bind(path: &Path) -> io::Result<Self> {
        use std::os::unix::fs::FileTypeExt;
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_socket() => std::fs::remove_file(path)?,
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "Agent socket path is not a socket",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener })
    }

    pub fn accept(&self, expected_agent: &str, timeout: Duration) -> io::Result<Connection> {
        let deadline = Instant::now() + timeout;
        let mut stream = loop {
            match self.listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Agent connection timed out; see agent.log",
                        ));
                    }
                    thread::sleep(Duration::from_millis(30));
                }
                Err(error) => return Err(error),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        match transport::read_message(&mut stream)? {
            Some(AgentWireMessage::Message(AgentMessage::AgentConnected {
                agent_id,
                capabilities,
            })) if agent_id == expected_agent => validate_capabilities(&capabilities)?,
            _ => return Err(io::Error::other("Agent handshake failed")),
        }
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        Ok(Connection {
            stream,
            reader: None,
        })
    }
}

pub struct Connection {
    stream: UnixStream,
    reader: Option<thread::JoinHandle<()>>,
}

impl Connection {
    pub fn send(&mut self, command: AgentCommand) -> io::Result<()> {
        transport::write_message(&mut self.stream, &AgentWireMessage::Command(command))
    }

    pub fn receive(
        &mut self,
        deliver: impl Fn(io::Result<Option<AgentMessage>>) -> bool + Send + 'static,
    ) -> io::Result<()> {
        if self.reader.is_some() {
            return Err(io::Error::other("Agent reader already started"));
        }
        let mut reader = self.stream.try_clone()?;
        self.reader = Some(thread::spawn(move || {
            loop {
                let message = match transport::read_message(&mut reader) {
                    Ok(Some(AgentWireMessage::Message(message))) => Ok(Some(message)),
                    Ok(Some(AgentWireMessage::Command(_))) => {
                        Err(io::Error::other("Agent sent a control-plane command"))
                    }
                    Ok(None) => Ok(None),
                    Err(error) => Err(error),
                };
                let done = !matches!(message, Ok(Some(_)));
                if !deliver(message) || done {
                    break;
                }
            }
        }));
        Ok(())
    }

    /// Half-close after control logic has confirmed cleanup/save, then drain facts to EOF.
    pub fn finish(&self) -> io::Result<()> {
        self.stream.shutdown(std::net::Shutdown::Write)
    }

    pub fn stop_receiving(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Read);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
        self.stop_receiving();
    }
}

fn validate_capabilities(capabilities: &AdapterCapabilities) -> io::Result<()> {
    let required = [
        (
            capabilities.pre_execution_interception,
            "preExecutionInterception",
        ),
        (
            capabilities.local_decision_evaluation,
            "localDecisionEvaluation",
        ),
        (
            capabilities.checkpoint_continuation,
            "checkpointContinuation",
        ),
        (
            capabilities.independent_candidate_session,
            "independentCandidateSession",
        ),
        (capabilities.cancel_completion, "cancelCompletion"),
        (capabilities.tool_result_injection, "toolResultInjection"),
    ];
    if let Some((_, name)) = required.into_iter().find(|(supported, _)| !supported) {
        return Err(io::Error::other(format!(
            "Agent Adapter lacks required capability {name}"
        )));
    }
    Ok(())
}

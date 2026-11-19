//! Agent listener, handshake, typed ingress and command delivery.
pub mod protocol;

use self::protocol::{AdapterCapabilities, AgentCommand, AgentMessage, AgentWireMessage};
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
            next_delivery: 0,
            receipts: std::sync::Arc::new((std::sync::Mutex::new(None), std::sync::Condvar::new())),
        })
    }
}

pub struct Connection {
    stream: UnixStream,
    reader: Option<thread::JoinHandle<()>>,
    next_delivery: u64,
    receipts: std::sync::Arc<(std::sync::Mutex<Option<u64>>, std::sync::Condvar)>,
}

impl Connection {
    /// A single ordered delivery worker owns this writer clone.
    pub fn delivery_writer(&self) -> io::Result<Self> {
        Ok(Self {
            stream: self.stream.try_clone()?,
            reader: None,
            next_delivery: self.next_delivery,
            receipts: self.receipts.clone(),
        })
    }

    pub fn send(&mut self, command: AgentCommand) -> io::Result<()> {
        self.send_cancellable(command, &std::sync::atomic::AtomicBool::new(false))
    }

    pub fn send_cancellable(
        &mut self,
        command: AgentCommand,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> io::Result<()> {
        self.next_delivery += 1;
        let id = self.next_delivery;
        let message = AgentWireMessage::Delivery { id, command };
        // Only receipt timeouts can be retried on this framed stream. A write
        // error may leave a partial frame, so close rather than replay bytes.
        for _ in 0..3 {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "delivery cancelled; previously sent frames may have executed",
                ));
            }
            if let Err(error) = transport::write_message(&mut self.stream, &message) {
                let _ = self.stream.shutdown(std::net::Shutdown::Both);
                return Err(error);
            }
            // Short waits let cancellation stop retries without another thread.
            let deadline = Instant::now() + Duration::from_millis(200);
            let mut receipt = self.receipts.0.lock().unwrap();
            while *receipt != Some(id)
                && Instant::now() < deadline
                && !cancelled.load(std::sync::atomic::Ordering::Acquire)
            {
                receipt = self
                    .receipts
                    .1
                    .wait_timeout(receipt, Duration::from_millis(5))
                    .unwrap()
                    .0;
            }
            if *receipt == Some(id) {
                return Ok(());
            }
        }
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "delivery cancelled; previously sent frames may have executed",
            ));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Agent did not acknowledge reception after 3 attempts",
        ))
    }

    pub fn receive(
        &mut self,
        deliver: impl Fn(io::Result<Option<AgentMessage>>) -> bool + Send + 'static,
    ) -> io::Result<()> {
        if self.reader.is_some() {
            return Err(io::Error::other("Agent reader already started"));
        }
        let mut reader = self.stream.try_clone()?;
        let receipts = self.receipts.clone();
        self.reader = Some(thread::spawn(move || loop {
            let message = match transport::read_message(&mut reader) {
                Ok(Some(AgentWireMessage::Message(message))) => Ok(Some(message)),
                Ok(Some(AgentWireMessage::Received { id })) => {
                    let mut receipt = receipts.0.lock().unwrap();
                    *receipt = Some(receipt.unwrap_or(0).max(id));
                    drop(receipt);
                    receipts.1.notify_all();
                    continue;
                }
                Ok(Some(AgentWireMessage::Command(_) | AgentWireMessage::Delivery { .. })) => {
                    Err(io::Error::other("Agent sent a control-plane command"))
                }
                Ok(None) => Ok(None),
                Err(error) => Err(error),
            };
            let done = !matches!(message, Ok(Some(_)));
            if !deliver(message) || done {
                break;
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
        if self.reader.is_some() {
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
            self.stop_receiving();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_stops_retries_but_cleanup_is_delivered() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let mut connection = Connection {
            stream,
            reader: None,
            next_delivery: 0,
            receipts: Arc::new((std::sync::Mutex::new(None), std::sync::Condvar::new())),
        };
        connection.receive(|_| true).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = cancelled.clone();
        let receiver = thread::spawn(move || {
            let _: AgentWireMessage = transport::read_message(&mut peer).unwrap().unwrap();
            token.store(true, Ordering::Release);
            let next: AgentWireMessage = transport::read_message(&mut peer).unwrap().unwrap();
            let AgentWireMessage::Delivery {
                id,
                command: AgentCommand::Cleanup { .. },
            } = next
            else {
                panic!("startup retried after cancellation")
            };
            transport::write_message(&mut peer, &AgentWireMessage::Received { id }).unwrap();
        });
        assert_eq!(
            connection
                .send_cancellable(
                    AgentCommand::Prompt {
                        branch_id: "b".into(),
                        prompt: "go".into()
                    },
                    &cancelled
                )
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
        connection
            .send(AgentCommand::Cleanup {
                branch_id: "b".into(),
                workspace: "/tmp/fake".into(),
                command_id: "cleanup".into(),
                generation: 0,
            })
            .unwrap();
        receiver.join().unwrap();
    }

    #[test]
    fn delivery_retries_same_id_until_receiver_acknowledges() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let mut connection = Connection {
            stream,
            reader: None,
            next_delivery: 0,
            receipts: std::sync::Arc::new((std::sync::Mutex::new(None), std::sync::Condvar::new())),
        };
        connection.receive(|_| true).unwrap();
        let receiver = thread::spawn(move || {
            let first: AgentWireMessage = transport::read_message(&mut peer).unwrap().unwrap();
            let second: AgentWireMessage = transport::read_message(&mut peer).unwrap().unwrap();
            assert_eq!(first, second);
            let AgentWireMessage::Delivery { id, .. } = second else {
                panic!("missing delivery envelope")
            };
            transport::write_message(&mut peer, &AgentWireMessage::Received { id }).unwrap();
        });
        connection
            .send(AgentCommand::Pause {
                branch_id: "branch".into(),
            })
            .unwrap();
        receiver.join().unwrap();
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

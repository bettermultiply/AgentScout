//! Adapter 会话：连接 control plane、收发消息。

use super::*;
use crate::protocol::contract::*;
use crate::protocol::transport;
use crate::runtime::backend::*;
use std::io;
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;

pub struct Adapter {
    pub adapter_instance: String,
    pub runtime: Arc<dyn Backend>,
}

impl Adapter {
    pub fn run(self, address: &str) -> io::Result<()> {
        let stream = TcpStream::connect(address)?;
        let mut reader = stream.try_clone()?;
        let (out, out_rx) = mpsc::channel::<AdapterMessage>();

        let mut writer = stream;
        thread::spawn(move || {
            for message in out_rx {
                if transport::write_message(&mut writer, &message).is_err() {
                    break;
                }
            }
        });

        let _ = out.send(AdapterMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            adapter_instance: self.adapter_instance,
            capabilities: self.runtime.capabilities(),
            backend_ids: self.runtime.backend_ids(),
        });

        let shared = Arc::new(Shared {
            commands: Some(commands::Commands::start(self.runtime.clone())?),
            preserving: std::sync::atomic::AtomicBool::new(false),
            runtime: self.runtime,
            out,
            queues: Mutex::new(HashMap::new()),
            seen: Mutex::new(Default::default()),
            results: Mutex::new(ResultCache::default()),
            result_signal: Condvar::new(),
            known: Mutex::new(HashMap::new()),
            agent: Mutex::new(None),
        });
        shared.clone().start_state_watcher();

        loop {
            match transport::read_message::<_, ControlMessage>(&mut reader) {
                Ok(Some(ControlMessage::Request(request))) => shared.dispatch(request),
                Ok(None) => {
                    eprintln!("adapter: control plane disconnected");
                    break;
                }
                Err(error) => {
                    eprintln!("adapter: read failed: {error}");
                    break;
                }
            }
        }
        shared.shutdown();
        Ok(())
    }
}


use super::*;
use crate::protocol::contract::*;
use crate::protocol::transport;
use crate::runtime::backend::*;
use std::io;
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;

pub struct RuntimeProxy {
    pub adapter_instance: String,
    pub runtime: Arc<BaseRuntime>,
}

impl RuntimeProxy {
    pub fn run(self, address: &str) -> io::Result<()> {
        let result = self.serve(address);
        if let Err(error) = self.runtime.shutdown() {
            eprintln!("runtime proxy: Runtime shutdown incomplete: {error}");
        }
        result
    }

    fn serve(&self, address: &str) -> io::Result<()> {
        let stream = TcpStream::connect(address)?;
        stream.set_nodelay(true)?;
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
            adapter_instance: self.adapter_instance.clone(),
            capabilities: self.runtime.capabilities(),
            backend_ids: self.runtime.backend_ids(),
        });

        let shared = Arc::new(Shared {
            commands: Some(commands::Commands::start(self.runtime.clone())?),
            runtime: self.runtime.clone(),
            out,
            queues: Mutex::new(HashMap::new()),
            seen: Mutex::new(Default::default()),
            results: Mutex::new(ResultCache::default()),
            result_signal: Condvar::new(),
            known: Mutex::new(HashMap::new()),
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
        for queue in shared.queues.lock().unwrap().values() {
            queue.close();
        }
        if let Some(commands) = &shared.commands {
            commands.stop();
        }
        Ok(())
    }
}

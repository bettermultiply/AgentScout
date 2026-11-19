//! Process signal forwarding for the Coordinator's input channel.
use super::ui;
use ascout::control_plane::coordination::execution::Event;
use std::io::Read;
#[cfg(test)]
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, Ordering},
    mpsc,
};
use std::{io, thread, time::Duration};

static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static INTERRUPT_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn request_shutdown(signal: libc::c_int) {
    // Signal handlers must not allocate, lock, or perform transport I/O.
    if signal == libc::SIGINT {
        INTERRUPT_REQUESTED.store(true, Ordering::Release);
    } else {
        SHUTDOWN_REQUESTED.store(true, Ordering::Release);
    }
    let fd = SIGNAL_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        unsafe {
            libc::write(fd, [1u8].as_ptr().cast(), 1);
        }
    }
}

pub fn install_shutdown_handlers() -> io::Result<()> {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = request_shutdown as *const () as usize;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        for signal in [libc::SIGINT, libc::SIGTERM] {
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

fn forward_signals(
    mut reader: UnixStream,
    shutdown: &AtomicBool,
    interrupt: &AtomicBool,
    sender: mpsc::Sender<Event>,
) {
    // The pipe is only a wake hint. Check pending intent periodically too:
    // a full pipe or a late flag must never strand the main loop in recv().
    let mut byte = [0];
    loop {
        if shutdown.swap(false, Ordering::AcqRel) {
            let _ = sender.send(Event::Request(ui::Request::Shutdown));
        }
        if interrupt.swap(false, Ordering::AcqRel) {
            let _ = sender.send(Event::Request(ui::Request::Interrupt));
        }
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(_) => break,
        }
    }
}

pub struct SignalForwarding {
    writer: UnixStream,
    worker: Option<thread::JoinHandle<()>>,
}

impl Drop for SignalForwarding {
    fn drop(&mut self) {
        SIGNAL_FD.store(-1, Ordering::Relaxed);
        // EOF cannot be lost when the nonblocking wake socket is full.
        let _ = self.writer.shutdown(std::net::Shutdown::Write);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn forward(events: mpsc::Sender<Event>) -> io::Result<SignalForwarding> {
    let (reader, writer) = UnixStream::pair()?;
    reader.set_read_timeout(Some(Duration::from_millis(20)))?;
    writer.set_nonblocking(true)?;
    let worker = thread::Builder::new().spawn(move || {
        forward_signals(reader, &SHUTDOWN_REQUESTED, &INTERRUPT_REQUESTED, events)
    })?;
    SIGNAL_FD.store(writer.as_raw_fd(), Ordering::Relaxed);
    Ok(SignalForwarding {
        writer,
        worker: Some(worker),
    })
}

#[cfg(test)]
mod signal_tests {
    use super::*;

    #[test]
    fn late_and_repeated_signals_do_not_need_another_pipe_byte() {
        let (mut reader, writer) = UnixStream::pair().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let shutdown = AtomicBool::new(false);
        let interrupt = AtomicBool::new(false);
        let (sender, receiver) = mpsc::channel();
        // Force the wake byte to be consumed before the pending flag is visible.
        (&writer).write_all(&[1]).unwrap();
        reader.read_exact(&mut [0]).unwrap();
        thread::scope(|scope| {
            scope.spawn(|| forward_signals(reader, &shutdown, &interrupt, sender));
            let result = || {
                for _ in 0..3 {
                    interrupt.store(true, Ordering::Release);
                    assert!(matches!(
                        receiver.recv_timeout(Duration::from_millis(500)),
                        Ok(Event::Request(ui::Request::Interrupt))
                    ));
                }
                shutdown.store(true, Ordering::Release);
                assert!(matches!(
                    receiver.recv_timeout(Duration::from_millis(500)),
                    Ok(Event::Request(ui::Request::Shutdown))
                ));
            };
            // Close even when an assertion fails, so the scoped worker cannot hang.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(result));
            writer.shutdown(std::net::Shutdown::Write).unwrap();
            if let Err(error) = outcome {
                std::panic::resume_unwind(error);
            }
        });
    }
}

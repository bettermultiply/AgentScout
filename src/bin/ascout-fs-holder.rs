#[path = "ascout-fs-holder/namespace.rs"]
mod namespace;
#[path = "ascout-fs-holder/protocol.rs"]
mod protocol;

use std::{io, path::PathBuf};

fn main() -> io::Result<()> {
    // Socket-only entry is also used by the protocol regression test.
    if let Some(socket) = std::env::args_os().nth(1) {
        protocol::serve(protocol::listen(&PathBuf::from(socket))?)
    } else {
        namespace::run()
    }
}

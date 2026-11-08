fn main() {
    if let Err(error) = ascout::runtime::backends::firecracker::guest_access::serve() {
        eprintln!("ascout-agent-access: {error}");
        std::process::exit(1);
    }
}

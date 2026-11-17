fn main() {
    if let Err(error) = ascout::runtime::backends::firecracker::guest_agent::serve() {
        eprintln!("ascout-guest-agent: {error}");
        std::process::exit(1);
    }
}

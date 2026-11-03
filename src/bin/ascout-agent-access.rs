fn main() {
    if let Err(error) = ascout::guest_access::serve() {
        eprintln!("ascout-agent-access: {error}");
        std::process::exit(1);
    }
}

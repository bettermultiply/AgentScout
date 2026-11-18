use std::path::PathBuf;

#[path = "ascout-runtime-adapter/config.rs"]
mod config;

fn main() {
    if let Err(error) = run() {
        eprintln!("ascout-runtime-adapter: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--config")) {
        return Err("usage: ascout-runtime-adapter --config <path>".into());
    }
    let path = PathBuf::from(arguments.next().ok_or("missing config path")?);
    if arguments.next().is_some() {
        return Err("unexpected command-line argument".into());
    }
    let (proxy, control_address) = config::assemble(&path)?;
    proxy.run(&control_address)?;
    Ok(())
}

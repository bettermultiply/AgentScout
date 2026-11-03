//! Compile once, then evaluate the artifact without loading the author YAML again.
use std::{
    env, fs,
    io::{self, Read},
    path::Path,
};

use ascout::policy::{CompiledPolicy, Decision, PolicyEngine};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    let engine = PolicyEngine::default();
    match args.first().map(String::as_str) {
        Some("compile") if args.len() == 3 => {
            let policy = engine.compile(Path::new(&args[1]))?;
            fs::write(&args[2], serde_json::to_vec_pretty(&policy)?)?;
            println!("{}", policy.digest());
        }
        Some("evaluate") if args.len() == 4 => {
            let policy: CompiledPolicy = serde_json::from_slice(&fs::read(&args[1])?)?;
            let decision: Decision = serde_json::from_value(args[2].clone().into())?;
            let bytes = if args[3] == "-" {
                let mut bytes = Vec::new();
                io::stdin().take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
                if bytes.len() > 1024 * 1024 {
                    return Err("Policy input exceeds 1 MiB".into());
                }
                bytes
            } else {
                fs::read(&args[3])?
            };
            let data = serde_json::from_slice(&bytes)?;
            println!("{}", engine.evaluate(&policy, decision, &data)?);
        }
        _ => return Err("usage: cargo run --example policy -- compile <policy.yaml> <artifact.json> | evaluate <artifact.json> <decision> <input.json>".into()),
    }
    Ok(())
}

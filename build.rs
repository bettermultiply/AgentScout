use std::{env, fs, path::Path};

fn embed(dir: &Path, sources: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            embed(&path, sources);
        } else if path.extension().is_some_and(|ext| ext == "py") {
            let name = path.strip_prefix("policy_lib").unwrap().to_str().unwrap();
            let absolute = fs::canonicalize(&path).unwrap();
            sources.push(format!(
                "({name:?}.into(), include_str!({absolute:?}).into()),"
            ));
        }
    }
}

fn main() {
    let dir = Path::new("policy_lib/ascout_policy");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut sources = Vec::new();
    embed(dir, &mut sources);
    sources.sort();
    fs::write(
        Path::new(&env::var_os("OUT_DIR").unwrap()).join("policy_sources.rs"),
        format!("BTreeMap::from([{}])", sources.join("\n")),
    )
    .unwrap();
}

use super::*;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::io::Write;

pub fn digest(path: &Path) -> io::Result<String> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}

pub fn publish(run_dir: &Path, generation: &Path, tree: &SpeculativeTree) -> io::Result<()> {
    let tree_path=generation.join("tree.json");
    write(&tree_path,&serde_json::to_vec_pretty(tree)?)?;
    let files=["tree.json","agent.json","runtime.json"].into_iter()
        .map(|name|Ok((name.to_string(),json!({"path":generation.join(name),"sha256":digest(&generation.join(name))?}))))
        .collect::<io::Result<serde_json::Map<String,Value>>>()?;
    write(&run_dir.join("recovery.json"),&serde_json::to_vec_pretty(&json!({"version":1,"files":files}))?)
}

pub fn load(run_dir: &Path) -> io::Result<(SpeculativeTree,std::path::PathBuf)> {
    let record:Value=serde_json::from_slice(&fs::read(run_dir.join("recovery.json"))?)?;
    if record["version"]!=1 {return Err(io::Error::other("Unsupported recovery version"));}
    let mut generation=None;
    for name in ["tree.json","agent.json","runtime.json"] {
        let file=&record["files"][name];
        let path=std::path::PathBuf::from(file["path"].as_str().ok_or_else(||io::Error::other("Missing recovery component"))?);
        let path=fs::canonicalize(path)?;
        if !path.starts_with(run_dir) || path.file_name().and_then(|s|s.to_str())!=Some(name) || file["sha256"]!=digest(&path)? {
            return Err(io::Error::other("Recovery component mismatch"));
        }
        let parent=path.parent().unwrap().to_path_buf();
        if generation.as_ref().is_some_and(|previous|previous!=&parent) {return Err(io::Error::other("Mixed recovery generations"));}
        generation=Some(parent);
    }
    let generation=generation.unwrap();
    Ok((serde_json::from_slice(&fs::read(generation.join("tree.json"))?)?,generation))
}

fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary=path.with_extension("tmp");
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    file.write_all(bytes)?;file.sync_all()?;
    if fs::read(&temporary)?!=bytes {return Err(io::Error::other("Recovery readback failed"));}
    fs::rename(temporary,path)?;
    fs::File::open(path.parent().unwrap())?.sync_all()
}

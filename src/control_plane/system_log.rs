//! Process-owned, fixed-format system runtime log.
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
};
#[derive(Clone)]
pub struct SystemLog {
    file: Arc<Mutex<File>>,
    warnings: Arc<Mutex<Vec<String>>>,
}
impl SystemLog {
    pub fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self {
            file: Arc::new(Mutex::new(
                OpenOptions::new().create(true).append(true).open(path)?,
            )),
            warnings: Default::default(),
        })
    }
    pub fn write(
        &self,
        level: &str,
        component: &str,
        session: Option<&str>,
        description: &str,
    ) -> io::Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let seconds = now.as_secs() as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe {
            libc::gmtime_r(&seconds, &mut tm);
        }
        let description = description.replace(['\r', '\n'], " ");
        let session = session.map(|s| format!(" session={s}")).unwrap_or_default();
        let line = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z {level} {component}{session} {description}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
            now.subsec_millis()
        );
        self.warnings.lock().unwrap().push(line.clone());
        writeln!(self.file.lock().unwrap(), "{line}")
    }
    pub fn history_failure(&self, session: &str, error: &dyn std::fmt::Display) {
        let description = format!("history incomplete: {error}");
        if let Err(error) = self.write("WARN", "ledger", Some(session), &description) {
            eprintln!("System log write failed: {error}; session={session} {description}");
        }
    }
    pub fn warnings(&self) -> Vec<String> {
        self.warnings.lock().unwrap().clone()
    }
}

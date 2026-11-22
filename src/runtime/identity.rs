
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::backend::Backend;

static NEXT_ENVIRONMENT: AtomicU64 = AtomicU64::new(1);

pub(super) fn allocate(runtime: &(impl Backend + ?Sized)) -> String {
    loop {
        let sequence = NEXT_ENVIRONMENT.fetch_add(1, Ordering::Relaxed);
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let identity = format!("env-{:x}-{epoch:x}-{sequence:x}", std::process::id());
        if runtime.state(&identity).is_none() {
            return identity;
        }
    }
}

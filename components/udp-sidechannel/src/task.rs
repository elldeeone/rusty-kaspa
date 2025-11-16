use std::{future::Future, thread};
use tokio::runtime::{Builder, Handle};

/// Spawns the provided future on the current Tokio runtime if one exists, otherwise spins up a
/// dedicated current-thread runtime inside a named thread.
pub fn spawn_detached<F>(name: &str, future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    if let Ok(handle) = Handle::try_current() {
        handle.spawn(future);
        return;
    }

    let thread_name = format!("udp-{name}");
    thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            Builder::new_current_thread().enable_all().build().expect("udp task runtime").block_on(future);
        })
        .expect("spawn udp task thread");
}

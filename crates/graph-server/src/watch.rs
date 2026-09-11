//! The one thread that rebuilds. It owns a recursive `notify` watcher on the
//! project root and the wake channel `/reload` pings; requests themselves
//! never rebuild, they only flip flags on the `Server`, so there is no lock
//! order between a poll and a re-index and `/reload` can never stall a poll.
//!
//! A batch of file events is done once 300 ms pass with no further source
//! event; Auto mode then rebuilds, Static mode marks the graph dirty. A wake
//! ping rebuilds regardless of mode. Events arriving during a rebuild queue
//! in the channel and open the next batch, so an edit made during a slow
//! re-index is not lost.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{RecursiveMode, Watcher};

use crate::handlers::{Mode, Server};

const QUIET: Duration = Duration::from_millis(300);

enum Msg {
    Fs(PathBuf),
    Wake,
}

pub fn spawn(root: PathBuf, server: Arc<Server>) -> Result<()> {
    let (tx, rx) = mpsc::channel::<Msg>();
    let fs_tx = tx.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            for path in event.paths {
                let _ = fs_tx.send(Msg::Fs(path));
            }
        }
    })
    .context("creating the file watcher")?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .with_context(|| format!("watching {}", root.display()))?;
    server.set_wake(move || {
        let _ = tx.send(Msg::Wake);
    });

    std::thread::Builder::new()
        .name("watch".into())
        .spawn(move || {
            // Dropping the watcher unregisters it, so it lives here for the
            // whole loop.
            let _watcher = watcher;
            run(&root, &server, &rx);
        })
        .context("spawning the watcher thread")?;
    Ok(())
}

fn run(root: &Path, server: &Server, rx: &mpsc::Receiver<Msg>) {
    let mut batch_open = false;
    loop {
        let msg = if batch_open {
            rx.recv_timeout(QUIET)
        } else {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match msg {
            Ok(Msg::Fs(path)) => batch_open |= is_source(root, &path),
            Ok(Msg::Wake) => {
                if server.take_request() {
                    batch_open = false;
                    rebuild(server);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                batch_open = false;
                match server.mode() {
                    Mode::Auto => rebuild(server),
                    Mode::Static => server.mark_dirty(),
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn rebuild(server: &Server) {
    // Whatever asked for this rebuild is satisfied by it; a queued wake ping
    // then finds nothing pending and does not rebuild twice.
    server.take_request();
    if let Err(e) = server.rebuild() {
        eprintln!("rebuild failed; keeping the previous graph: {e:#}");
    }
}

/// Same rule as the SCIP index freshness check: under `root`, any component
/// starting with `.` or equal to `target` or `node_modules` is not a source.
/// A path outside `root` is not a source either.
pub fn is_source(root: &Path, changed: &Path) -> bool {
    let Ok(rel) = changed.strip_prefix(root) else { return false };
    rel.components().all(|c| {
        let name = c.as_os_str().to_string_lossy();
        !name.starts_with('.') && name != "target" && name != "node_modules"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_source_filters_like_the_index_freshness_rule() {
        let root = Path::new("/repo");
        for hidden in [".git/HEAD", "target/x", "a/node_modules/b", "src/.hidden/x.rs", "src/.x.rs.swp"] {
            assert!(!is_source(root, &root.join(hidden)), "{hidden} must not count");
        }
        assert!(is_source(root, &root.join("src/x.rs")));
        assert!(is_source(root, &root.join("targets/x.rs")), "only an exact `target` component is build output");
        assert!(!is_source(root, Path::new("/elsewhere/src/x.rs")));
    }
}

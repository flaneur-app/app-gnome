// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tokio runtime that runs network and storage work, on its own threads.
//!
//! GTK widgets live on the main thread and must never wait on the network. Work goes to this
//! runtime with [`spawn`]; results come back to the GTK main loop by awaiting the returned
//! [`JoinHandle`](tokio::task::JoinHandle) or a [`tokio::sync::watch`] channel from
//! `glib::spawn_future_local`. Both are executor-independent futures, so the main loop can await
//! them directly.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

/// The shared runtime, started on first use.
pub fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .thread_name("flaneur-worker")
            .enable_all()
            .build()
            .expect("the tokio runtime starts")
    })
}

/// Runs a future on the runtime.
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime().spawn(future)
}

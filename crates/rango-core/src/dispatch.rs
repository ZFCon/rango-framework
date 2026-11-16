//! `dispatch.Signal`.
//!
//! ```
//! use rango_core::dispatch::{ConnectOptions, Signal};
//!
//! static GREETED: Signal<String, usize> = Signal::new();
//!
//! # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
//! GREETED.connect_sync(|d| d.args.len(), ConnectOptions::default());
//! let responses = GREETED.send(None, "hello".to_owned()).await;
//! assert_eq!(responses[0].1, 5);
//! # });
//! ```
//!
//! Differences: senders are identified by a label string (a model's
//! `"app_label.ModelName"`, a type name, ...) instead of object identity; there are no weak
//! references (Rust ownership: `disconnect` explicitly); a signal is typed by its argument
//! struct `A` and the receivers' return type `R`.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;

/// What a receiver is called with: the sender label plus the signal's named arguments.
#[derive(Debug)]
pub struct Dispatch<A> {
    pub sender: Option<String>,
    pub args: A,
}

/// Handle returned by `connect`, usable with [`Signal::disconnect`].
pub type ReceiverId = u64;

/// Options of `Signal.connect(receiver, sender=None, dispatch_uid=None)`.
#[derive(Debug, Clone, Default)]
pub struct ConnectOptions {
    /// Only receive signals from this sender (`None` = any sender).
    pub sender: Option<String>,
    /// Identifies the receiver so connecting it twice registers it once.
    pub dispatch_uid: Option<String>,
}

impl ConnectOptions {
    pub fn sender(sender: impl Into<String>) -> Self {
        Self {
            sender: Some(sender.into()),
            dispatch_uid: None,
        }
    }

    pub fn uid(mut self, uid: impl Into<String>) -> Self {
        self.dispatch_uid = Some(uid.into());
        self
    }
}

type Handler<A, R> = Arc<dyn Fn(Arc<Dispatch<A>>) -> BoxFuture<'static, R> + Send + Sync>;

struct Entry<A, R> {
    id: ReceiverId,
    sender: Option<String>,
    uid: Option<String>,
    handler: Handler<A, R>,
}

/// A signal with argument type `A`; each receiver returns an `R`.
pub struct Signal<A, R = ()> {
    receivers: RwLock<Vec<Entry<A, R>>>,
    next_id: AtomicU64,
}

impl<A, R> Default for Signal<A, R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A, R> Signal<A, R> {
    pub const fn new() -> Self {
        Self {
            receivers: RwLock::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }
}

impl<A: Send + Sync + 'static, R: Send + 'static> Signal<A, R> {
    /// `connect()` with an async receiver.
    pub fn connect<F, Fut>(&self, receiver: F, options: ConnectOptions) -> ReceiverId
    where
        F: Fn(Arc<Dispatch<A>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
    {
        let mut receivers = self.receivers.write().expect("signal lock poisoned");
        if let Some(uid) = &options.dispatch_uid
            && let Some(existing) = receivers
                .iter()
                .find(|e| e.uid.as_ref() == Some(uid) && e.sender == options.sender)
        {
            return existing.id;
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        receivers.push(Entry {
            id,
            sender: options.sender,
            uid: options.dispatch_uid,
            handler: Arc::new(move |d| receiver(d).boxed()),
        });
        id
    }

    /// `connect()` with a plain function.
    pub fn connect_sync<F>(&self, receiver: F, options: ConnectOptions) -> ReceiverId
    where
        F: Fn(&Dispatch<A>) -> R + Send + Sync + 'static,
    {
        let receiver = Arc::new(receiver);
        self.connect(
            move |d| {
                let receiver = Arc::clone(&receiver);
                async move { receiver(&d) }
            },
            options,
        )
    }

    /// `disconnect()` by handle.
    pub fn disconnect(&self, id: ReceiverId) -> bool {
        let mut receivers = self.receivers.write().expect("signal lock poisoned");
        let before = receivers.len();
        receivers.retain(|e| e.id != id);
        receivers.len() != before
    }

    /// `disconnect(dispatch_uid=..., sender=...)`.
    pub fn disconnect_uid(&self, uid: &str, sender: Option<&str>) -> bool {
        let mut receivers = self.receivers.write().expect("signal lock poisoned");
        let before = receivers.len();
        receivers.retain(|e| !(e.uid.as_deref() == Some(uid) && e.sender.as_deref() == sender));
        receivers.len() != before
    }

    fn live_receivers(&self, sender: Option<&str>) -> Vec<(ReceiverId, Handler<A, R>)> {
        self.receivers
            .read()
            .expect("signal lock poisoned")
            .iter()
            .filter(|e| e.sender.is_none() || e.sender.as_deref() == sender)
            .map(|e| (e.id, Arc::clone(&e.handler)))
            .collect()
    }

    /// `has_listeners(sender)`.
    pub fn has_listeners(&self, sender: Option<&str>) -> bool {
        !self.live_receivers(sender).is_empty()
    }

    /// `send()`: call every matching receiver in connection order; a panic propagates.
    pub async fn send(&self, sender: Option<&str>, args: A) -> Vec<(ReceiverId, R)> {
        let dispatch = Arc::new(Dispatch {
            sender: sender.map(str::to_owned),
            args,
        });
        let mut out = Vec::new();
        for (id, handler) in self.live_receivers(sender) {
            out.push((id, handler(Arc::clone(&dispatch)).await));
        }
        out
    }

    /// `send_robust()`: like [`send`](Self::send) but a panicking receiver is reported as `Err`
    /// (with the panic message) and the remaining receivers still run.
    pub async fn send_robust(
        &self,
        sender: Option<&str>,
        args: A,
    ) -> Vec<(ReceiverId, Result<R, String>)> {
        let dispatch = Arc::new(Dispatch {
            sender: sender.map(str::to_owned),
            args,
        });
        let mut out = Vec::new();
        for (id, handler) in self.live_receivers(sender) {
            let result = AssertUnwindSafe(handler(Arc::clone(&dispatch)))
                .catch_unwind()
                .await;
            out.push((
                id,
                result.map_err(|p| {
                    p.downcast_ref::<&str>()
                        .map(|s| (*s).to_owned())
                        .or_else(|| p.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "receiver panicked".to_owned())
                }),
            ));
        }
        out
    }
}

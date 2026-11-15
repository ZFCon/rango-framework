//! `core.signals`: request lifecycle signals.

use crate::dispatch::Signal;

/// Sent by the handler when a request starts (sender: handler name).
pub static REQUEST_STARTED: Signal<()> = Signal::new();
/// Sent when the response has been fully handled.
pub static REQUEST_FINISHED: Signal<()> = Signal::new();
/// Sent when an unhandled error occurs while handling a request (argument: its message).
pub static GOT_REQUEST_EXCEPTION: Signal<String> = Signal::new();

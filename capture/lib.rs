// Not a hook binary. This is the "capture" half of the pipeline as a library: normalize
// a HookEvent into one shape and log it. Whatever binary is actually invoked by Claude
// Code (currently `recall`, for every event) calls into this after redacting.

pub mod error;
pub mod normalizer;

pub use error::CaptureError;
pub use normalizer::{normalize, NormalizedEvent};

// TODO: persist the normalized step once the capture store (db/log) exists.
pub fn record_step(event: &NormalizedEvent) {
    eprintln!(
        "capture: session={} event={} tool={:?} payload={}",
        event.session_id, event.hook_event_name, event.tool_name, event.payload
    );
}

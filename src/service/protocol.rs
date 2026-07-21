//! Re-exports of the wire protocol shared with the GUI.
//!
//! The concrete types live in `logilightshow-api` so a client crate can
//! depend on the small API crate without pulling in USB, capture, or the
//! service runtime.

pub use logilightshow_api::protocol::{
    ClientCommand, ClientRequest, RequestId, RequestResult, ServerMessage,
};

//! What a client is told of a failure: internal details (paths, task
//! failures) stay in the manager's logs, over HTTP as over the WebSocket.

use bergadeck_protocol::ErrorCode;

/// Message of an internal error, its detail logged.
pub(super) const INTERNAL_MESSAGE: &str = "internal error, see the manager logs";

/// The message a client receives for a failure `code` described by
/// `detail`: the detail itself, except for an internal error, logged then
/// replaced by [`INTERNAL_MESSAGE`].
pub(super) fn client_message(code: ErrorCode, detail: impl Into<String>, context: &str) -> String {
    let detail = detail.into();
    if code == ErrorCode::Internal {
        tracing::error!(error = %detail, context, "request failed");
        INTERNAL_MESSAGE.to_owned()
    } else {
        detail
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_internal_details_are_hidden() {
        let path = "cannot access /home/me/config/devices/X: denied";
        assert_eq!(
            client_message(ErrorCode::Internal, path, "test"),
            INTERNAL_MESSAGE
        );
        assert_eq!(
            client_message(ErrorCode::InvalidInput, "no input with index 9", "test"),
            "no input with index 9"
        );
    }
}

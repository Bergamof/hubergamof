//! `hubergamof native-host`: the program the browser starts for the
//! extension (native messaging), relaying its frames to the manager and the
//! manager's back. It speaks the same framing on both sides and keeps no
//! state of its own.

use std::io;

use tokio::io::{AsyncRead, AsyncWrite};

use super::wire::{MAX_FRAME_BYTES, read_frame, write_frame};

/// Copies frames from `from` to `to` until `from` ends cleanly.
async fn forward<R, W>(mut from: R, mut to: W) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    while let Some(frame) = read_frame(&mut from, MAX_FRAME_BYTES).await? {
        write_frame(&mut to, &frame).await?;
    }
    Ok(())
}

/// Relays the browser (`browser_in`, `browser_out`) to the manager
/// (`manager_in`, `manager_out`) until either side ends.
///
/// # Errors
///
/// Fails when a stream fails or carries an oversized or truncated frame.
pub async fn relay<BI, BO, MI, MO>(
    browser_in: BI,
    browser_out: BO,
    manager_in: MI,
    manager_out: MO,
) -> io::Result<()>
where
    BI: AsyncRead + Unpin,
    BO: AsyncWrite + Unpin,
    MI: AsyncRead + Unpin,
    MO: AsyncWrite + Unpin,
{
    tokio::select! {
        outcome = forward(browser_in, manager_out) => outcome,
        outcome = forward(manager_in, browser_out) => outcome,
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncWriteExt, duplex};

    use super::*;

    #[tokio::test]
    async fn frames_go_both_ways_until_the_browser_leaves() {
        let (mut browser_side, relay_browser) = duplex(1024);
        let (mut manager_side, relay_manager) = duplex(1024);
        let (browser_in, browser_out) = tokio::io::split(relay_browser);
        let (manager_in, manager_out) = tokio::io::split(relay_manager);
        let relaying = tokio::spawn(relay(browser_in, browser_out, manager_in, manager_out));

        write_frame(&mut browser_side, b"{\"to\":\"manager\"}")
            .await
            .unwrap();
        let received = read_frame(&mut manager_side, 64).await.unwrap().unwrap();
        assert_eq!(received, b"{\"to\":\"manager\"}");

        write_frame(&mut manager_side, b"{\"to\":\"browser\"}")
            .await
            .unwrap();
        let received = read_frame(&mut browser_side, 64).await.unwrap().unwrap();
        assert_eq!(received, b"{\"to\":\"browser\"}");

        browser_side.shutdown().await.unwrap();
        relaying.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn an_oversized_frame_ends_the_relay_with_an_error() {
        let (mut browser_side, relay_browser) = duplex(64);
        let (_manager_side, relay_manager) = duplex(64);
        let (browser_in, browser_out) = tokio::io::split(relay_browser);
        let (manager_in, manager_out) = tokio::io::split(relay_manager);
        let relaying = tokio::spawn(relay(browser_in, browser_out, manager_in, manager_out));

        browser_side.write_all(&[255, 255, 255, 255]).await.unwrap();
        assert!(relaying.await.unwrap().is_err());
    }
}

//! N-01 / SV-13 end to end with a real ntfy server and a real Android emulator (driven by `tests/e2e/ntfy-emulator.sh`).
//! Not part of `cargo test`: it only runs when asked (`--ignored`) and serves until the stop file appears.
//!
//! It starts a server on the address in ATOMIC_TEST_BIND, creates the user "emu" (the app signs in with the one-time login
//! link written to LINK_FILE) and the user "sender". Whenever the file SEND_FILE appears, "sender" posts a direct message to "emu"
//! (and the file is removed). When the file says "offline" the server first forgets the phone's live connection, so the only way the
//! message can reach the phone is the content-free wake-up through ntfy.
mod common;
use common::*;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn serve_for_the_emulator_ntfy_run() {
    let s = TestServer::start().await;
    let r = s.admin.create_user("emu", "Emu Phone", Some("password-123")).await.unwrap();
    let emu_id = r.user.user.id.clone();
    let link = s.admin.login_link(&emu_id).await.unwrap();
    let (_sid, sender) = s.user("sender").await;
    let dm = sender.open_dm(&emu_id).await.unwrap();
    std::fs::write(std::env::var("URL_FILE").expect("URL_FILE"), &s.url).unwrap();
    std::fs::write(std::env::var("LINK_FILE").expect("LINK_FILE"), &link.link).unwrap();
    let (send, stop) = (std::env::var("SEND_FILE").expect("SEND_FILE"), std::env::var("STOP_FILE").expect("STOP_FILE"));
    let mut n = 0;
    while !std::path::Path::new(&stop).exists() {
        if std::path::Path::new(&send).exists() {
            // "offline" in the file: the server treats the phone as having no live connection, so the message can only
            // reach it through the ntfy wake-up (the app then reconnects and syncs).
            if std::fs::read_to_string(&send).map(|t| t.contains("offline")).unwrap_or(false) {
                s.st.hub.forget_user(&emu_id);
            }
            let _ = std::fs::remove_file(&send);
            n += 1;
            let _ = sender.send_text(&dm, &format!("Wake-up test message {n}"), client_core::SendOptions::default());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

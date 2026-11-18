//! Not a test: starts a real in-process server with a believable team (users, channels, threads,
//! reactions, pins, a picture, unread messages) for the screenshot script. Ignored by default.
//!
//!   SEED_URL_FILE=/tmp/url cargo test -p atomic-client-core --test seed_demo -- --ignored --nocapture
//!
//! It writes the server URL to `SEED_URL_FILE`, then serves until `SEED_STOP_FILE` exists.
mod common;
use client_core::SendOptions;
use common::*;
use proto::*;
use std::time::Duration;

/// A small gradient PNG without extra crates (stored deflate blocks).
/// A 3 second speech-like WAV (a tone whose loudness rises and falls in syllables).
fn voice_wav() -> Vec<u8> {
    let rate = 16_000u32;
    let samples: Vec<i16> = (0..rate * 3)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let env = ((t * 9.0).sin().abs() * (1.0 - (t / 3.0 - 0.5).abs())).powf(1.2);
            (env * 16000.0 * (t * 2.0 * std::f32::consts::PI * 180.0).sin()) as i16
        })
        .collect();
    let dl = (samples.len() * 2) as u32;
    let mut w = b"RIFF".to_vec();
    w.extend((36 + dl).to_le_bytes());
    w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes());
    w.extend(1u16.to_le_bytes());
    w.extend(1u16.to_le_bytes());
    w.extend(rate.to_le_bytes());
    w.extend((rate * 2).to_le_bytes());
    w.extend(2u16.to_le_bytes());
    w.extend(16u16.to_le_bytes());
    w.extend(b"data");
    w.extend(dl.to_le_bytes());
    for x in samples {
        w.extend(x.to_le_bytes());
    }
    w
}

/// A small JPEG gradient used as a video poster frame.
fn poster_jpeg() -> String {
    use base64::Engine as _;
    let img = image::RgbImage::from_fn(320, 180, |x, y| image::Rgb([(40 + x / 3) as u8, (90 + y / 2) as u8, 160]));
    let mut out = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 70).encode_image(&img).unwrap();
    base64::engine::general_purpose::STANDARD.encode(out)
}

/// A one-page PDF (a heading, a few lines and a coloured block) so the demo shows a real first-page preview.
fn demo_pdf() -> Vec<u8> {
    let content = "0.1 0.55 0.45 rg 40 700 515 6 re f BT /F1 26 Tf 40 740 Td (Project brief) Tj ET BT /F1 13 Tf 40 670 Td 18 TL (Goal: ship the onboarding flow by Friday.) ' (Scope: sign-in, invite link, first message.) ' (Owner: Bob   Reviewer: Alice) ' ET 0.93 0.95 0.97 rg 40 480 515 150 re f 0.1 0.55 0.45 rg 40 480 8 150 re f BT /F1 14 Tf 60 600 Td 18 TL (Risks) ' (- Spacing on the login screen) ' (- Dark mode contrast) ' ET";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = vec![];
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objs.len() + 1, xref).as_bytes());
    out
}

fn png(w: u32, h: u32) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc(&body).to_be_bytes());
    }
    let mut raw = vec![];
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            raw.extend([(x * 255 / w) as u8, (y * 255 / h) as u8, 160]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut i = 0;
    while i < raw.len() {
        let n = (raw.len() - i).min(65535);
        z.push(if i + n == raw.len() { 1 } else { 0 });
        z.extend((n as u16).to_le_bytes());
        z.extend((!(n as u16)).to_le_bytes());
        z.extend(&raw[i..i + n]);
        i += n;
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = vec![];
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn proto_now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

#[tokio::test]
#[ignore]
async fn seed_and_serve() {
    let s = TestServer::start().await;
    let (alice_id, alice) = s.user("alice").await;
    let (bob_id, bob) = s.user("bob").await;
    let (carol_id, carol) = s.user("carol").await;
    let (dave_id, dave) = s.user("dave").await;
    s.admin.update_user(&alice_id, &UpdateUserRequest { display_name: Some("Alice Nguyen".into()), ..Default::default() }).await.ok();
    // Alice is an admin so the screenshots show the admin controls (Add people, rename, archive).
    s.admin.set_role(&alice_id, Role::Admin).await.ok();
    // Carol has Privacy options: Allow (sign-out controls visible); everyone else is Disallow.
    s.admin.update_user(&carol_id, &UpdateUserRequest { privacy_controls_enabled: Some(true), ..Default::default() }).await.ok();
    let general = make_channel(&s, "general", &[&alice_id, &bob_id, &carol_id, &dave_id]).await;
    let design = make_channel(&s, "design", &[&alice_id, &bob_id, &carol_id]).await;
    let random = make_channel(&s, "random", &[&alice_id, &dave_id]).await;
    for (c, ch) in [(&bob, &general), (&carol, &general), (&dave, &general), (&bob, &design), (&carol, &design), (&dave, &random), (&alice, &general), (&alice, &design), (&alice, &random)] {
        wait_conv(c, ch).await;
    }
    let m1 = say(&bob, &general, "Morning all. *Release 1.4* goes out at 3 pm today.").await;
    say(&carol, &general, "Checklist before we ship:\n- changelog reviewed\n- _screenshots_ updated\n- ~old banner~ removed").await;
    say(&dave, &general, "Run `cargo test --workspace` first, then:\n```\nmake release\nmake publish\n```").await;
    let m4 = say(&alice, &general, "Thanks @bob. Docs are here https://example.com/docs/1.4 if anyone needs them.").await;
    say(&bob, &general, "> Thanks @bob\nAnytime!").await;
    bob.react(&m4.id, "\u{1F44D}", true).await.unwrap();
    carol.react(&m4.id, "\u{1F44D}", true).await.unwrap();
    dave.react(&m4.id, "\u{1F389}", true).await.unwrap();
    alice.react(&m1.id, "\u{1F680}", true).await.unwrap();
    alice.send_thread_reply(&general, &m1.id, "Confirmed, the build is green.").unwrap();
    bob.send_thread_reply(&general, &m1.id, "Great. Tagging now.").unwrap();
    let img = s.dir.path().join("mockup.png");
    std::fs::write(&img, png(480, 270)).unwrap();
    carol.send_files(&design, "New onboarding mockup", &[img.clone()], SendOptions::default()).unwrap();
    let pdf = s.dir.path().join("brief.pdf");
    std::fs::write(&pdf, demo_pdf()).unwrap();
    bob.send_files(&design, "Brief for review", &[pdf], SendOptions::default()).unwrap();
    let wav = s.dir.path().join("voice-message-1.wav");
    std::fs::write(&wav, voice_wav()).unwrap();
    carol.send_files(&design, "", &[wav], SendOptions::default()).unwrap();
    let mp4 = s.dir.path().join("walkthrough.mp4");
    std::fs::write(&mp4, b"not a real video, only a demo").unwrap();
    let vmeta = proto::AttachmentMeta { duration_ms: Some(48_000), width: Some(320), height: Some(180), poster_b64: Some(poster_jpeg()), ..Default::default() };
    bob.send_files_with(&design, "Walkthrough of the new flow", &[(mp4, Some(vmeta))], SendOptions::default()).unwrap();
    wait_msg(&alice, &design, "Brief for review").await;
    // Unread in design for Alice, a DM and a group.
    say(&bob, &design, "Can you check the spacing on the *login screen*? @alice").await;
    let dm = bob.open_dm(&alice_id).await.unwrap();
    wait_conv(&alice, &dm).await;
    say(&bob, &dm, "Lunch at 12:30?").await;
    let grp = s.admin.create_group("Launch crew", &[alice_id.clone(), bob_id.clone(), dave_id.clone()]).await.unwrap().conversation.id;
    wait_conv(&dave, &grp).await;
    say(&dave, &grp, "Banner is live.").await;
    say(&dave, &random, "Anyone up for a coffee run?").await;
    // Read state: Alice has read general only.
    alice.mark_read(&general).await.unwrap();
    // Pin and bookmark as an admin.
    let _: Vec<Pin> = s.admin.call(reqwest::Method::PUT, &format!("/conversations/{general}/pins/{}", m1.id), None).await.unwrap();
    let _: Vec<Bookmark> = s
        .admin
        .call(reqwest::Method::POST, &format!("/conversations/{general}/bookmarks"), Some(serde_json::to_value(BookmarkRequest { title: "Release notes".into(), url: Some("https://example.com/notes".into()), attachment_id: None, position: None }).unwrap()))
        .await
        .unwrap();
    let _ = (carol_id, dave_id);

    // An app (bot) with a form in #general, a direct message with a Home tab, a scheduled message and a draft.
    let bot = s.admin.create_bot("expense_bot", "Expense Bot", "Collects and approves expense claims", &[general.clone()]).await.unwrap();
    let http = reqwest::Client::new();
    let tg = |method: &'static str, body: serde_json::Value| {
        let (http, url, token) = (http.clone(), s.url.clone(), bot.token.clone());
        async move {
            let v: serde_json::Value = http.post(format!("{url}/bot{token}/{method}")).json(&body).send().await.unwrap().json().await.unwrap();
            assert_eq!(v["ok"], true, "{v}");
            v["result"].clone()
        }
    };
    tg(
        "sendMessage",
        serde_json::json!({"chat_id": general, "text": "Taxi receipt from Dave needs approval ($42.10).", "form": {
            "id": "expense", "title": "Expense claim #42", "description": "Taxi to the airport, 14 November",
            "fields": [
                {"id": "ref", "type": "text", "label": "Reference", "required": true, "placeholder": "INV-123"},
                {"id": "note", "type": "multiline", "label": "Note"},
                {"id": "cat", "type": "select", "label": "Category", "options": [{"value": "travel", "label": "Travel"}, {"value": "tools", "label": "Tools"}]},
                {"id": "urgent", "type": "boolean", "label": "Urgent", "placeholder": "Needs a decision today"}],
            "buttons": [{"id": "approve", "label": "Approve", "style": "primary"}, {"id": "reject", "label": "Reject", "style": "danger", "form": {"id": "why", "title": "Why reject?", "fields": [{"id": "reason", "type": "text", "label": "Reason", "required": true}]}}],
            "submit_label": "Send response"}}),
    )
    .await;
    let bdm = alice.open_dm(&bot.user.id).await.unwrap();
    wait_conv(&alice, &bdm).await;
    tg("sendMessage", serde_json::json!({"chat_id": bdm, "text": "Hi! I log receipts and ask for approvals. Open my **Home** tab."})).await;
    tg(
        "setAppHome",
        serde_json::json!({"home": {"blocks": [
            {"type": "header", "text": "Expense Bot"},
            {"type": "text", "text": "3 claims waiting for approval. Totals this month: $412.60."},
            {"type": "divider"},
            {"type": "button", "id": "report", "label": "Send me this month's report", "style": "primary"},
            {"type": "form", "form": {"id": "newclaim", "title": "New claim", "description": "File an expense by hand", "fields": [{"id": "amount", "type": "text", "label": "Amount", "required": true}], "submit_label": "File claim"}}]}}),
    )
    .await;
    // AI search is on (pointing at nothing: the screenshots never run a query) so the toggle shows.
    s.admin.add_ai_provider(&AiProviderInput { name: "Demo".into(), kind: "anthropic".into(), api_key: "sk-demo-0000".into(), base_url: Some("http://127.0.0.1:9".into()), ..Default::default() }).await.unwrap();
    let due = proto_now_ms() + 2 * 3_600_000;
    alice.schedule_message(&general, "Reminder: retro starts at 4 pm.", due, None).await.unwrap();
    alice.save_draft(&design, None, "Notes on the login screen spacing: tighten the gap under the logo");
    // For the emulator: a one-time login link for Alice (the app opens it and signs in).
    if let Ok(f) = std::env::var("SEED_LINK_FILE") {
        let l = s.admin.login_link(&alice_id).await.unwrap();
        std::fs::write(f, l.link).unwrap();
    }
    std::fs::write(std::env::var("SEED_URL_FILE").expect("SEED_URL_FILE"), &s.url).unwrap();
    let stop = std::env::var("SEED_STOP_FILE").expect("SEED_STOP_FILE");
    while !std::path::Path::new(&stop).exists() {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[test]
fn demo_png_is_a_valid_png() {
    let p = png(48, 27);
    assert_eq!(&p[1..4], b"PNG");
    if let Ok(dir) = std::env::var("PNG_DUMP") {
        std::fs::write(dir, p).unwrap();
    }
}

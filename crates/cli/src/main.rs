//! `atomic`: headless Atomic Chat client built on client-core.
//!
//! Every command prints plain text, or JSON with `--json`. Sign-in state lives in the data
//! directory (`--data-dir`, default `~/.local/share/atomic-chat-cli`).

use client_core::admin::AdminClient;
use client_core::logic::{datefmt, markup};
use client_core::{Client, ClientConfig, ClientEvent, SendOptions};
use proto::*;
use std::path::PathBuf;
use std::time::Duration;

const HELP: &str = "atomic - Atomic Chat command line client

USAGE: atomic [--data-dir DIR] [--json] [--insecure-http] <command> [args]

Account
  login <server> <username> <password>      sign in (https required; --insecure-http for test servers)
  link <pasted-link-or-code> [server]       sign in with an admin's one-time link
  logout                                    sign out and erase local data
  whoami                                    show the signed-in user and connection details
  sessions                                  list this account's devices
Chats
  chats                                     list conversations with unread counts
  users                                     list people
  dm <username>                             open (or create) a direct message, prints its id
  history <conv> [--older N]                show messages (N extra older pages)
  send <conv> <text...>                     send a message (waits until the server accepts it)
  upload <conv> <file> [caption]            send a file
  download <conv> <message-id|latest> <dest-dir>
  edit <message-id> <text...>               edit your message (edit rule applies)
  delete <message-id>                       delete your message (tombstone)
  react <message-id> <emoji> [--remove]
  read <conv>                               mark a conversation as read
  search <query...>                         search messages you can see
  listen [seconds]                          sync and print live events
Admin (needs an admin or owner account; uses --server/--admin-user/--admin-password)
  admin create-user <username> <display name> [password]
  admin disable <username> | enable <username>
  admin reset-password <username> [new-password]
  admin clear-history <username>
  admin link <username>
  admin create-channel <name> <username>...
  admin storage | storage-test <json> | storage-migrate <json> | storage-verify
Conversation ids can be given as a prefix.";

struct Opts {
    data_dir: PathBuf,
    json: bool,
    insecure: bool,
    server: Option<String>,
    admin_user: String,
    admin_password: String,
}

#[tokio::main]
async fn main() {
    let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).try_init();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut o = Opts {
        data_dir: std::env::var("ATOMIC_CLI_DIR").map(PathBuf::from).unwrap_or_else(|_| {
            std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share/atomic-chat-cli")).unwrap_or_else(|_| PathBuf::from(".atomic-chat-cli"))
        }),
        json: false,
        insecure: std::env::var("ATOMIC_INSECURE_HTTP").is_ok(),
        server: std::env::var("ATOMIC_SERVER").ok(),
        admin_user: std::env::var("ATOMIC_ADMIN_USER").unwrap_or_else(|_| "admin".into()),
        admin_password: std::env::var("ATOMIC_ADMIN_PASSWORD").unwrap_or_default(),
    };
    // Global flags may appear anywhere before the command.
    let mut rest = vec![];
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--data-dir" if i + 1 < args.len() => {
                o.data_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--server" if i + 1 < args.len() => {
                o.server = Some(args[i + 1].clone());
                i += 2;
            }
            "--admin-user" if i + 1 < args.len() => {
                o.admin_user = args[i + 1].clone();
                i += 2;
            }
            "--admin-password" if i + 1 < args.len() => {
                o.admin_password = args[i + 1].clone();
                i += 2;
            }
            "--json" => {
                o.json = true;
                i += 1;
            }
            "--insecure-http" => {
                o.insecure = true;
                i += 1;
            }
            _ => {
                rest.push(args[i].clone());
                i += 1;
            }
        }
    }
    args = rest;
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        println!("{HELP}");
        return;
    }
    if let Err(e) = run(&o, &args).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn open(o: &Opts) -> anyhow::Result<Client> {
    let mut cfg = ClientConfig::new(&o.data_dir);
    cfg.allow_insecure_http = o.insecure;
    cfg.device_name = "atomic-cli".into();
    Ok(Client::open(cfg)?)
}

fn pretty<T: serde::Serialize>(o: &Opts, v: &T, text: impl FnOnce() -> String) {
    if o.json {
        println!("{}", serde_json::to_string(v).unwrap_or_default());
    } else {
        println!("{}", text());
    }
}

/// Starts sync and waits until the first catch-up completes (or `secs` pass).
async fn online(c: &Client, secs: u64) -> anyhow::Result<()> {
    anyhow::ensure!(c.signed_in(), "not signed in; run `atomic login` first");
    c.start();
    for _ in 0..(secs * 10) {
        if c.connection() == client_core::ConnState::Online {
            return Ok(());
        }
        if !c.signed_in() {
            anyhow::bail!("signed out by the server");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("could not connect to the server")
}

fn resolve_conv(c: &Client, s: &str) -> anyhow::Result<String> {
    let convs = c.conversations();
    if let Some(x) = convs.iter().find(|x| x.conversation.id == s) {
        return Ok(x.conversation.id.clone());
    }
    let m: Vec<_> = convs.iter().filter(|x| x.conversation.id.starts_with(s) || c.conversation_title(x).eq_ignore_ascii_case(s)).collect();
    match m.len() {
        1 => Ok(m[0].conversation.id.clone()),
        0 => anyhow::bail!("no conversation matches '{s}'"),
        _ => anyhow::bail!("'{s}' matches several conversations"),
    }
}

async fn admin(o: &Opts) -> anyhow::Result<AdminClient> {
    let server = o.server.clone().ok_or_else(|| anyhow::anyhow!("--server is required for admin commands"))?;
    Ok(AdminClient::login(&server, &o.admin_user, &o.admin_password).await?)
}

async fn user_id(a: &AdminClient, name: &str) -> anyhow::Result<String> {
    a.users().await?.into_iter().find(|u| u.user.username.eq_ignore_ascii_case(name)).map(|u| u.user.id).ok_or_else(|| anyhow::anyhow!("no user '{name}'"))
}

async fn run(o: &Opts, args: &[String]) -> anyhow::Result<()> {
    let cmd = args[0].as_str();
    let a = |n: usize| -> anyhow::Result<&str> { args.get(n).map(String::as_str).ok_or_else(|| anyhow::anyhow!("missing argument; see `atomic help`")) };
    match cmd {
        "login" => {
            let c = open(o)?;
            let me = c.login(a(1)?, a(2)?, a(3)?, None).await?;
            pretty(o, &me, || format!("signed in as {} ({})", me.user.display_name, me.user.username));
        }
        "link" => {
            let c = open(o)?;
            let me = c.login_with_link(a(1)?, args.get(2).map(String::as_str), None).await?;
            pretty(o, &me, || format!("signed in as {} ({}); set a password with `atomic passwd`", me.user.display_name, me.user.username));
        }
        "passwd" => {
            let c = open(o)?;
            c.change_password(args.get(2).map(String::as_str), a(1)?).await?;
            println!("password changed");
        }
        "logout" => {
            let c = open(o)?;
            c.logout().await?;
            println!("signed out; local data erased");
        }
        "whoami" => {
            let c = open(o)?;
            let s = c.session().ok_or_else(|| anyhow::anyhow!("not signed in"))?;
            let me = c.refresh_me().await?;
            pretty(o, &me, || format!("{} ({}) on {}, role {}", me.user.display_name, me.user.username, s.server, me.user.role.as_str()));
        }
        "sessions" => {
            let c = open(o)?;
            let v = c.sessions().await?;
            pretty(o, &v, || v.iter().map(|s| format!("{}{} {} last seen {}", if s.current { "* " } else { "  " }, s.device_name, s.client_version, datefmt::format_date(s.last_seen_ms, 0))).collect::<Vec<_>>().join("\n"));
        }
        "chats" => {
            let c = open(o)?;
            online(&c, 20).await?;
            let v = c.conversations();
            pretty(o, &v, || {
                v.iter()
                    .map(|x| format!("{}  {:<28} {:<8} unread {}", &x.conversation.id[..8], c.conversation_title(x), x.conversation.kind.as_str(), x.unread))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        "users" => {
            let c = open(o)?;
            online(&c, 20).await?;
            let v = c.users();
            pretty(o, &v, || v.iter().map(|u| format!("{:<20} {}", u.username, u.display_name)).collect::<Vec<_>>().join("\n"));
        }
        "dm" => {
            let c = open(o)?;
            online(&c, 20).await?;
            let who = a(1)?;
            let u = c.users().into_iter().find(|u| u.username.eq_ignore_ascii_case(who)).ok_or_else(|| anyhow::anyhow!("no such user"))?;
            let id = c.open_dm(&u.id).await?;
            println!("{id}");
        }
        "history" => {
            let c = open(o)?;
            online(&c, 20).await?;
            let conv = resolve_conv(&c, a(1)?)?;
            let pages: usize = args.iter().position(|x| x == "--older").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(0);
            for _ in 0..pages {
                if c.load_older(&conv, None).await? == 0 {
                    break;
                }
            }
            let tl = c.timeline(&conv, 500, None);
            pretty(o, &tl.iter().map(|t| &t.message).collect::<Vec<_>>(), || {
                tl.iter()
                    .map(|t| {
                        let m = &t.message;
                        let body = if m.deleted { "Message deleted".to_string() } else { markup::to_plain(&m.body) };
                        let files: String = m.attachments.iter().map(|f| format!(" [file: {}]", f.name)).collect();
                        format!("{} {} {}: {}{}{}", datefmt::format_date(m.ts, 0), datefmt::format_time(m.ts, 0, false), c.display_name(&m.sender_id), body, files, if m.is_edited() { " (edited)" } else { "" })
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        "send" => {
            let c = open(o)?;
            online(&c, 20).await?;
            let conv = resolve_conv(&c, a(1)?)?;
            let text = args[2..].join(" ");
            let cid = c.send_text(&conv, &text, SendOptions::default())?;
            wait_sent(&c, &conv, &cid).await?;
            println!("sent");
        }
        "upload" => {
            let c = open(o)?;
            online(&c, 20).await?;
            let conv = resolve_conv(&c, a(1)?)?;
            let cid = c.send_files(&conv, args.get(3).map(String::as_str).unwrap_or(""), &[PathBuf::from(a(2)?)], SendOptions::default())?;
            wait_sent(&c, &conv, &cid).await?;
            println!("uploaded");
        }
        "download" => {
            let c = open(o)?;
            online(&c, 20).await?;
            let conv = resolve_conv(&c, a(1)?)?;
            let tl = c.timeline(&conv, 500, None);
            let m = if a(2)? == "latest" {
                tl.iter().rev().find(|t| !t.message.attachments.is_empty()).map(|t| t.message.clone())
            } else {
                tl.iter().find(|t| t.message.id.starts_with(a(2).unwrap_or(""))).map(|t| t.message.clone())
            }
            .ok_or_else(|| anyhow::anyhow!("no such message with a file"))?;
            let att = m.attachments.first().ok_or_else(|| anyhow::anyhow!("message has no file"))?;
            let p = c.download_attachment(att).await?;
            let dest = PathBuf::from(a(3)?).join(&att.name);
            std::fs::create_dir_all(dest.parent().unwrap())?;
            std::fs::copy(&p, &dest)?;
            println!("{}", dest.display());
        }
        "edit" => {
            let c = open(o)?;
            let id = full_id(&c, a(1)?)?;
            let m = c.edit_message(&id, &args[2..].join(" ")).await?;
            pretty(o, &m, || format!("edited (version {})", m.version));
        }
        "delete" => {
            let c = open(o)?;
            let id = full_id(&c, a(1)?)?;
            c.delete_message(&id).await?;
            println!("deleted");
        }
        "react" => {
            let c = open(o)?;
            let id = full_id(&c, a(1)?)?;
            c.react(&id, a(2)?, !args.iter().any(|x| x == "--remove")).await?;
            println!("ok");
        }
        "read" => {
            let c = open(o)?;
            online(&c, 20).await?;
            let conv = resolve_conv(&c, a(1)?)?;
            c.mark_read(&conv).await?;
            println!("marked read");
        }
        "search" => {
            let c = open(o)?;
            let r = c.search(&args[1..].join(" ")).await?;
            pretty(o, &r, || r.hits.iter().map(|h| format!("{} {}: {}", &h.message.conv_id[..8], c.display_name(&h.message.sender_id), h.snippet)).collect::<Vec<_>>().join("\n"));
        }
        "listen" => {
            let c = open(o)?;
            online(&c, 20).await?;
            let mut rx = c.subscribe();
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
            let end = tokio::time::Instant::now() + Duration::from_secs(secs);
            while let Ok(Ok(ev)) = tokio::time::timeout_at(end, rx.recv()).await {
                match ev {
                    ClientEvent::Incoming { message, sender_name, .. } => println!("{sender_name}: {}", markup::to_plain(&message.body)),
                    ClientEvent::Typing { user_id, .. } => println!("{} is typing...", c.display_name(&user_id)),
                    ClientEvent::SignedOut { message, .. } => {
                        println!("{message}");
                        break;
                    }
                    other => {
                        if !o.json {
                            tracing_debug(&other);
                        }
                    }
                }
            }
        }
        "admin" => admin_cmd(o, args).await?,
        _ => anyhow::bail!("unknown command '{cmd}'; try `atomic help`"),
    }
    Ok(())
}

fn tracing_debug(_e: &ClientEvent) {}

fn full_id(c: &Client, s: &str) -> anyhow::Result<String> {
    // Accept a unique prefix of a cached message id.
    for cs in c.conversations() {
        for t in c.timeline(&cs.conversation.id, 500, None) {
            if t.message.id.starts_with(s) {
                return Ok(t.message.id);
            }
        }
    }
    Ok(s.to_string())
}

async fn wait_sent(c: &Client, conv: &str, cid: &str) -> anyhow::Result<()> {
    for _ in 0..1200 {
        let tl = c.timeline(conv, 50, None);
        match tl.iter().find(|t| t.message.client_id.as_deref() == Some(cid)) {
            Some(t) if !t.local => return Ok(()),
            Some(t) if t.state == client_core::SendState::Failed => anyhow::bail!("failed: {}", t.error.clone().unwrap_or_default()),
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    anyhow::bail!("timed out waiting for the server")
}

async fn admin_cmd(o: &Opts, args: &[String]) -> anyhow::Result<()> {
    let a = admin(o).await?;
    let sub = args.get(1).map(String::as_str).unwrap_or("");
    let p = |n: usize| -> anyhow::Result<&str> { args.get(n).map(String::as_str).ok_or_else(|| anyhow::anyhow!("missing argument")) };
    match sub {
        "create-user" => {
            let r = a.create_user(p(2)?, p(3)?, args.get(4).map(String::as_str)).await?;
            pretty(o, &r, || match &r.login_link {
                Some(l) => format!("created {}\none-time login link (valid 7 days):\n{}", r.user.user.username, l.link),
                None => format!("created {}", r.user.user.username),
            });
        }
        "disable" | "enable" => {
            let id = user_id(&a, p(2)?).await?;
            a.set_disabled(&id, sub == "disable").await?;
            println!("{sub}d");
        }
        "reset-password" => {
            let id = user_id(&a, p(2)?).await?;
            let r = a.reset_password(&id, args.get(3).map(String::as_str)).await?;
            match r.login_link {
                Some(l) => println!("password reset; one-time login link:\n{}", l.link),
                None => println!("password reset"),
            }
        }
        "clear-history" => {
            let id = user_id(&a, p(2)?).await?;
            let r = a.clear_history(&id).await?;
            pretty(o, &r, || format!("backup {} written; {} messages hidden, {} files moved to backup, {} sessions ended", r.backup_key, r.messages_hidden, r.attachments_moved, r.sessions_revoked));
        }
        "link" => {
            let id = user_id(&a, p(2)?).await?;
            println!("{}", a.login_link(&id).await?.link);
        }
        "create-channel" => {
            let mut ids = vec![];
            for n in &args[3..] {
                ids.push(user_id(&a, n).await?);
            }
            let c = a.create_channel(p(2)?, &ids).await?;
            println!("{}", c.conversation.id);
        }
        "storage" => {
            let s = a.storage().await?;
            println!("{}", serde_json::to_string_pretty(&s)?);
        }
        "storage-test" => {
            let cfg: StorageConfig = serde_json::from_str(p(2)?)?;
            let r = a.test_storage(&cfg).await?;
            println!("{} {}", if r.ok { "OK" } else { "FAILED" }, r.message);
        }
        "storage-migrate" => {
            let cfg: StorageConfig = serde_json::from_str(p(2)?)?;
            let j = a.migrate_storage(&cfg, true).await?;
            let j = a.wait_job(&j.id).await?;
            println!("{}: {}", j.state, j.message);
        }
        "storage-verify" => {
            let j = a.verify_storage(args.iter().any(|x| x == "--deep")).await?;
            let j = a.wait_job(&j.id).await?;
            println!("{}: {}", j.state, j.message);
        }
        _ => anyhow::bail!("unknown admin command"),
    }
    Ok(())
}

//! Desktop integrations done through standard command line tools, so the app needs no extra native
//! libraries: Secret Service keyring (`secret-tool`), notifications (`notify-send`), opening files
//! (`xdg-open`), clipboard (`wl-copy`/`xclip`/`xsel`), file chooser (`zenity`/`kdialog`).

use client_core::error::Result;
use client_core::secrets::{FileSecretStore, SecretStore};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Tokens in the user's keyring (GNOME Keyring, KWallet via the Secret Service API).
pub struct KeyringStore;

impl KeyringStore {
    /// True when `secret-tool` exists and a store/lookup/clear round trip works.
    pub fn works() -> bool {
        let s = KeyringStore;
        let probe = format!("probe-{}", std::process::id());
        if s.set(&probe, "ok").is_err() {
            return false;
        }
        let ok = matches!(s.get(&probe), Ok(Some(v)) if v == "ok");
        let _ = s.delete(&probe);
        ok
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        // "atomic-chat" is the service name of 1.0; entries there are still readable.
        for service in ["superchat", "atomic-chat"] {
            let out = Command::new("secret-tool").args(["lookup", "service", service, "key", key]).stderr(Stdio::null()).output();
            if let Ok(o) = out {
                if o.status.success() && !o.stdout.is_empty() {
                    return Ok(Some(String::from_utf8_lossy(&o.stdout).to_string()));
                }
            }
        }
        Ok(None)
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        let mut child = Command::new("secret-tool")
            .args(["store", "--label=SuperChat", "service", "superchat", "key", key])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| client_core::ClientError::Local(format!("keyring unavailable: {e}")))?;
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(value.as_bytes());
        }
        let st = child.wait().map_err(|e| client_core::ClientError::Local(e.to_string()))?;
        if st.success() {
            Ok(())
        } else {
            Err(client_core::ClientError::Local("keyring refused the secret".into()))
        }
    }
    fn delete(&self, key: &str) -> Result<()> {
        for service in ["superchat", "atomic-chat"] {
            let _ = Command::new("secret-tool").args(["clear", "service", service, "key", key]).stderr(Stdio::null()).status();
        }
        Ok(())
    }
}

/// Windows Credential Manager through the `keyring` crate.
#[cfg(windows)]
pub struct WindowsStore;

#[cfg(windows)]
impl SecretStore for WindowsStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        for service in ["superchat", "atomic-chat"] {
            let e = keyring::Entry::new(service, key).map_err(|e| client_core::ClientError::Local(e.to_string()))?;
            match e.get_password() {
                Ok(v) => return Ok(Some(v)),
                Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(client_core::ClientError::Local(e.to_string())),
            }
        }
        Ok(None)
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        keyring::Entry::new("superchat", key).and_then(|e| e.set_password(value)).map_err(|e| client_core::ClientError::Local(e.to_string()))
    }
    fn delete(&self, key: &str) -> Result<()> {
        let _ = keyring::Entry::new("superchat", key).and_then(|e| e.delete_credential());
        Ok(())
    }
}

/// Keyring when it works (Credential Manager on Windows, Secret Service elsewhere), otherwise a 0600 file in the app
/// data dir (documented in DECISIONS.md).
pub fn desktop_secrets(data_dir: &Path) -> Arc<dyn SecretStore> {
    #[cfg(windows)]
    {
        let probe = format!("probe-{}", std::process::id());
        let w = WindowsStore;
        if w.set(&probe, "ok").is_ok() && matches!(w.get(&probe), Ok(Some(v)) if v == "ok") {
            let _ = w.delete(&probe);
            return Arc::new(WindowsStore);
        }
    }
    if cfg!(not(windows)) && proto::env_var("ATOMIC_NO_KEYRING").is_err() && KeyringStore::works() {
        Arc::new(KeyringStore)
    } else {
        Arc::new(FileSecretStore::new(data_dir.join("client").join("secrets.json")))
    }
}

/// `silent`: the notification daemon must not play its own sound (the app plays a chosen one, or none).
pub fn notify(title: &str, body: &str, silent: bool) {
    #[cfg(windows)]
    let _ = silent;
    #[cfg(windows)]
    {
        // A balloon from a short PowerShell script; best effort.
        let esc = |t: &str| t.replace('\'', "''");
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; $n = New-Object System.Windows.Forms.NotifyIcon; $n.Icon = [System.Drawing.SystemIcons]::Information; $n.Visible = $true; $n.ShowBalloonTip(5000, '{}', '{}', 'Info'); Start-Sleep -Seconds 6; $n.Dispose()",
            esc(title),
            esc(body)
        );
        let _ = Command::new("powershell").args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script]).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        return;
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("notify-send");
        cmd.args(["--app-name=SuperChat", "--icon=mail-unread"]);
        if silent {
            cmd.arg("--hint=boolean:suppress-sound:true");
        }
        let _ = cmd.args([title, body]).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }
}

/// The system "open" command: xdg-open on Linux, `cmd /C start` on Windows.
fn opener(target: &std::ffi::OsStr) {
    #[cfg(windows)]
    let _ = Command::new("cmd").args(["/C", "start", ""]).arg(target).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    #[cfg(not(windows))]
    let _ = Command::new("xdg-open").arg(target).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

pub fn open_path(p: &Path) {
    opener(p.as_os_str());
}

pub fn open_url(u: &str) {
    if u.starts_with("http://") || u.starts_with("https://") || u.starts_with("mailto:") {
        opener(std::ffi::OsStr::new(u));
    }
}

pub fn copy_text(t: &str) -> bool {
    #[cfg(windows)]
    if let Ok(mut ch) = Command::new("clip").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        if let Some(mut si) = ch.stdin.take() {
            let _ = si.write_all(t.as_bytes());
        }
        return true;
    }
    for (bin, args) in [("wl-copy", vec![]), ("xclip", vec!["-selection", "clipboard"]), ("xsel", vec!["--clipboard", "--input"])] {
        if let Ok(mut ch) = Command::new(bin).args(&args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            if let Some(mut si) = ch.stdin.take() {
                let _ = si.write_all(t.as_bytes());
            }
            return true;
        }
    }
    false
}

/// Native file chooser through zenity or kdialog; None when cancelled or neither is installed.
pub fn pick_file() -> Option<PathBuf> {
    pick_file_of(false)
}

/// The same chooser, showing video files first (M-31 "Upload video").
pub fn pick_video() -> Option<PathBuf> {
    pick_file_of(true)
}

fn pick_file_of(video: bool) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let filter = if video { "$d.Filter = 'Videos|*.mp4;*.mkv;*.webm;*.mov;*.avi;*.m4v|All files|*.*';" } else { "" };
        let script = format!("Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; {filter} if ($d.ShowDialog() -eq 'OK') {{ $d.FileName }}");
        if let Ok(o) = Command::new("powershell").args(["-NoProfile", "-Command", &script]).stderr(Stdio::null()).output() {
            let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
            return if p.is_empty() { None } else { Some(PathBuf::from(p)) };
        }
        return None;
    }
    #[cfg(not(windows))]
    for (bin, args) in [
        (
            "zenity",
            if video {
                vec!["--file-selection", "--title=Choose a video to send", "--file-filter=Videos | *.mp4 *.mkv *.webm *.mov *.avi *.m4v", "--file-filter=All files | *"]
            } else {
                vec!["--file-selection", "--title=Choose a file to send"]
            },
        ),
        ("kdialog", if video { vec!["--getopenfilename", ".", "*.mp4 *.mkv *.webm *.mov *.avi *.m4v|Videos"] } else { vec!["--getopenfilename", "."] }),
    ] {
        if let Ok(o) = Command::new(bin).args(&args).stderr(Stdio::null()).output() {
            if o.status.success() {
                let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !p.is_empty() {
                    return Some(PathBuf::from(p));
                }
            }
            return None;
        }
    }
    None
}

/// Start-on-login via an XDG autostart entry.
pub fn set_autostart(on: bool, minimised: bool) {
    #[cfg(windows)]
    {
        const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        if on {
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
            let val = format!("\"{exe}\"{}", if minimised { " --minimised" } else { "" });
            let _ = Command::new("reg").args(["add", KEY, "/v", "SuperChat", "/t", "REG_SZ", "/d", &val, "/f"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        } else {
            let _ = Command::new("reg").args(["delete", KEY, "/v", "SuperChat", "/f"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
        return;
    }
    #[cfg(not(windows))]
    set_autostart_xdg(on, minimised);
}

#[cfg(not(windows))]
fn set_autostart_xdg(on: bool, minimised: bool) {
    let Some(home) = std::env::var_os("HOME") else { return };
    let dir = PathBuf::from(home).join(".config/autostart");
    let f = dir.join("superchat.desktop");
    if !on {
        let _ = std::fs::remove_file(f);
        return;
    }
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "superchat".into());
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(f, format!("[Desktop Entry]\nType=Application\nName=SuperChat\nExec={exe}{}\nX-GNOME-Autostart-enabled=true\n", if minimised { " --minimised" } else { "" }));
}

/// F-02: files (copied in a file manager) or an image on the clipboard, as local paths ready to attach. Works on X11,
/// Wayland and Windows through the `arboard` crate, with no helper program; empty when the clipboard holds neither.
pub fn clipboard_attachments(dir: &Path) -> Vec<PathBuf> {
    #[cfg(not(target_os = "android"))]
    {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if let Ok(files) = cb.get().file_list() {
                let files: Vec<PathBuf> = files.into_iter().filter(|p| p.is_file()).collect();
                if !files.is_empty() {
                    return files;
                }
            }
            if let Ok(img) = cb.get_image() {
                if let Some(buf) = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned()) {
                    let _ = std::fs::create_dir_all(dir);
                    let out = dir.join(format!("pasted-{}.png", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)));
                    if buf.save_with_format(&out, image::ImageFormat::Png).is_ok() {
                        return vec![out];
                    }
                }
            }
        }
    }
    clipboard_image(dir).into_iter().collect()
}

/// Image on the clipboard as a PNG file (F-02 paste), through wl-paste, xclip or PowerShell. None when there is no image.
pub fn clipboard_image(dir: &Path) -> Option<PathBuf> {
    let out = dir.join(format!("pasted-{}.png", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)));
    #[cfg(windows)]
    {
        let script = format!("Add-Type -AssemblyName System.Windows.Forms; $i = [System.Windows.Forms.Clipboard]::GetImage(); if ($i) {{ $i.Save('{}'); 'ok' }}", out.display());
        let o = Command::new("powershell").args(["-NoProfile", "-Command", &script]).stderr(Stdio::null()).output().ok()?;
        return if String::from_utf8_lossy(&o.stdout).contains("ok") && out.is_file() { Some(out) } else { None };
    }
    #[cfg(not(windows))]
    for (bin, args) in [("wl-paste", vec!["--type", "image/png"]), ("xclip", vec!["-selection", "clipboard", "-t", "image/png", "-o"])] {
        if let Ok(o) = Command::new(bin).args(&args).stderr(Stdio::null()).output() {
            if o.status.success() && o.stdout.starts_with(&[0x89, b'P', b'N', b'G']) {
                std::fs::write(&out, &o.stdout).ok()?;
                return Some(out);
            }
        }
    }
    #[allow(unreachable_code)]
    None
}

/// Plays an audio file (voice message) with whatever player is installed; returns the child so it can be stopped.
pub fn play_audio(p: &Path) -> Option<std::process::Child> {
    #[cfg(windows)]
    {
        let script = format!("(New-Object Media.SoundPlayer '{}').PlaySync()", p.display());
        return Command::new("powershell").args(["-NoProfile", "-Command", &script]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok();
    }
    #[cfg(not(windows))]
    for (bin, args) in [("ffplay", vec!["-nodisp", "-autoexit", "-loglevel", "quiet"]), ("mpv", vec!["--no-video", "--really-quiet"]), ("paplay", vec![]), ("aplay", vec!["-q"])] {
        if let Ok(ch) = Command::new(bin).args(&args).arg(p).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            return Some(ch);
        }
    }
    #[allow(unreachable_code)]
    None
}

/// M-31: recording the screen into an MP4 with `ffmpeg` (X11 and Windows). Needs ffmpeg on the PATH; where it is missing,
/// or on a Wayland session (the screen cannot be grabbed without the portal), the "Record screen" entry is not shown.
pub struct ScreenRecorder {
    child: std::process::Child,
    pub path: PathBuf,
    pub started: Instant,
}

pub fn screen_record_available() -> bool {
    let ffmpeg = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let has = Command::new(ffmpeg).arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    #[cfg(windows)]
    {
        has
    }
    #[cfg(not(windows))]
    {
        has && std::env::var("DISPLAY").is_ok() && std::env::var("XDG_SESSION_TYPE").map(|t| t != "wayland").unwrap_or(true)
    }
}

/// Starts recording. `limit_bytes` stops the recording by itself at the server's upload limit.
pub fn screen_record_start(dir: &Path, limit_bytes: u64) -> Option<ScreenRecorder> {
    let _ = std::fs::create_dir_all(dir);
    let name = format!("screen-recording-{}.mp4", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    let path = dir.join(name);
    let mut cmd = Command::new(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
    cmd.args(["-y", "-hide_banner", "-loglevel", "error"]);
    #[cfg(windows)]
    cmd.args(["-f", "gdigrab", "-framerate", "15", "-i", "desktop"]);
    #[cfg(not(windows))]
    {
        let display = std::env::var("DISPLAY").ok()?;
        cmd.args(["-f", "x11grab", "-framerate", "15", "-i", &display]);
    }
    cmd.args(["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-movflags", "+faststart"]);
    if limit_bytes > 0 {
        // Leave a margin for the container's index.
        cmd.args(["-fs", &(limit_bytes.saturating_sub(limit_bytes / 20)).to_string()]);
    }
    // ffmpeg reads "q" on its stdin as "finish the file".
    let child = cmd.arg(&path).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
    Some(ScreenRecorder { child, path, started: Instant::now() })
}

/// Stops and returns the finished file (None when nothing usable was written).
pub fn screen_record_stop(mut r: ScreenRecorder) -> Option<PathBuf> {
    use std::io::Write;
    if let Some(mut stdin) = r.child.stdin.take() {
        let _ = stdin.write_all(b"q\n");
    }
    let end = Instant::now() + Duration::from_secs(8);
    loop {
        match r.child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                // It did not finish in time: the file has no index and cannot be played.
                let _ = r.child.kill();
                let _ = r.child.wait();
                let _ = std::fs::remove_file(&r.path);
                return None;
            }
        }
    }
    std::fs::metadata(&r.path).ok().filter(|m| m.len() > 1024).map(|_| r.path)
}

//! Update checks in the UI (Settings > About, rail and Home banner). The logic lives in `client_core::update`; this file
//! turns its status into Slint properties, starts the automatic checks and runs the platform's install step.

use crate::{platform, toast, AppWindow, Hooks, Sh};
use client_core::update::{self, Phase, Platform, Status, Updater};
use slint::ComponentHandle;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What happened after the download that the update status itself does not know.
#[derive(Default, Clone)]
struct Local {
    /// The file whose install step already ran (so it runs once per download, not on every refresh).
    handled: Option<PathBuf>,
    title: String,
    note: String,
    /// "" or the label of the action button after a download.
    action: String,
    command: String,
    /// The install step failed: shown as an error with "Try again".
    error: Option<String>,
    /// AppImage replaced: restart into this file.
    restart: Option<PathBuf>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn mb(n: u64) -> String {
    format!("{:.1} MB", n as f64 / 1_048_576.0)
}

fn ago(ms: i64, now: i64) -> String {
    if ms <= 0 {
        return "never".into();
    }
    let m = ((now - ms) / 60_000).max(0);
    match m {
        0 => "just now".into(),
        1 => "1 minute ago".into(),
        2..=59 => format!("{m} minutes ago"),
        60..=119 => "1 hour ago".into(),
        120..=2879 => format!("{} hours ago", m / 60),
        _ => format!("{} days ago", m / 1440),
    }
}

/// The text of the "Version" line: `Version 1.3.1 (build abc1234)` on the desktop, `Version 1.3.1 (code 8)` on Android.
pub fn version_long(code: Option<u32>) -> String {
    let v = client_core::CLIENT_VERSION;
    if let Some(c) = code {
        format!("Version {v} (code {c})")
    } else if !client_core::BUILD_COMMIT.is_empty() {
        format!("Version {v} (build {})", client_core::BUILD_COMMIT)
    } else {
        format!("Version {v}")
    }
}

fn asset_line(st: &Status) -> String {
    match &st.pick {
        Some(p) => format!("{} ({})", st.platform.label(), mb(p.asset.size)),
        None => String::new(),
    }
}

/// Pushes the status into the window.
fn show(ui: &AppWindow, st: &Status, local: &Local, auto: bool, now: i64) {
    let rel_v = st.release.as_ref().map(|r| r.version.clone()).unwrap_or_default();
    let s = |a: &str| a.to_string();
    let checked = if st.last_check_ms > 0 { format!("Last checked {}.", ago(st.last_check_ms, now)) } else { "Not checked yet.".to_string() };
    let (phase, title, detail, primary, command) = match st.phase {
        Phase::Idle => (s("idle"), s("Check for new versions"), checked, String::new(), String::new()),
        Phase::Checking => (s("checking"), s("Checking for updates..."), String::new(), String::new(), String::new()),
        Phase::UpToDate => (s("uptodate"), s("You're up to date"), format!("SuperChat {} is the latest version. {}", st.current, checked), String::new(), String::new()),
        Phase::Available => {
            let hint = st.manual_hint();
            let detail = format!("You have {}. {}{}", st.current, asset_line(st), if hint.is_empty() { String::new() } else { format!("\n{hint}") });
            let primary = if st.pick.is_some() { s("Download & install") } else if st.platform == Platform::Flatpak { String::new() } else { s("Open the download page") };
            (s("available"), format!("Update available: version {rel_v}"), detail, primary, String::new())
        }
        Phase::Downloading => (s("downloading"), format!("Downloading version {rel_v}"), String::new(), String::new(), String::new()),
        Phase::Ready => match &local.error {
            Some(e) => (s("failed"), e.clone(), String::new(), s("Try again"), String::new()),
            None => (
                s("ready"),
                if local.title.is_empty() { format!("Version {rel_v} downloaded and verified") } else { local.title.clone() },
                local.note.clone(),
                local.action.clone(),
                local.command.clone(),
            ),
        },
        Phase::Failed => (s("failed"), st.error.clone(), String::new(), s("Try again"), String::new()),
    };
    ui.set_upd_phase(phase.into());
    ui.set_upd_title(title.into());
    ui.set_upd_detail(detail.into());
    ui.set_upd_notes(st.release.as_ref().map(|r| r.notes.clone()).unwrap_or_default().into());
    ui.set_upd_primary(primary.into());
    ui.set_upd_command(command.into());
    let (p, text) = if st.total > 0 { (st.done as f32 / st.total as f32, format!("{} of {}", mb(st.done), mb(st.total))) } else { (0.0, mb(st.done)) };
    ui.set_upd_progress(p);
    ui.set_upd_progress_text(text.into());
    ui.set_upd_auto(auto);
    ui.set_upd_can_skip(true);
    let banner = st.show_banner(now);
    ui.set_upd_banner(banner);
    ui.set_upd_banner_text(if banner { format!("Update available: version {rel_v}") } else { String::new() }.into());
}

/// The install step after a verified download. Android hands the APK to the system installer; the other platforms follow
/// `docs/DECISIONS.md` (1.3.1).
fn install(ui: &AppWindow, hooks: &Hooks, st: &Status, local: &mut Local) {
    let Some(file) = st.file.clone() else { return };
    local.handled = Some(file.clone());
    local.error = None;
    local.restart = None;
    local.command.clear();
    let v = st.release.as_ref().map(|r| r.version.clone()).unwrap_or_default();
    match st.platform {
        Platform::Android => match &hooks.install_apk {
            Some(f) => {
                f(&file);
                local.title = "Installing...".into();
                local.note = "Android's installer opens next. Confirm it to finish. If Android asks to allow installs from SuperChat, turn that on and come back: the install continues by itself.".into();
                local.action = "Install again".into();
            }
            None => {
                local.error = Some("This device cannot install the update from inside the app.".into());
            }
        },
        Platform::LinuxAppImage => {
            let target = std::env::var("APPIMAGE").map(PathBuf::from).unwrap_or_default();
            match update::replace_appimage(&file, &target) {
                Ok(bak) => {
                    local.title = format!("Version {v} is installed");
                    local.note = format!("Restart SuperChat to use it. The previous version was kept as {}.", bak.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                    local.action = "Restart now".into();
                    local.restart = Some(target);
                }
                Err(e) => local.error = Some(format!("Could not replace the AppImage: {e}. The new file is at {}.", file.display())),
            }
        }
        Platform::LinuxDeb => {
            local.command = update::deb_install_command(&file);
            if platform::has_deb_installer() {
                platform::open_path(&file);
                local.title = "Opening your software installer".into();
                local.note = format!("{} was downloaded and verified. If nothing opened, run this command in a terminal:", file.display());
                local.action = "Open installer again".into();
            } else {
                local.title = "Downloaded. Install it from a terminal".into();
                local.note = format!("{} was downloaded and verified. This computer has no graphical package installer set up, so run:", file.display());
                local.action = String::new();
            }
        }
        Platform::WindowsInstalled => match std::process::Command::new(&file).spawn() {
            Ok(_) => {
                local.title = "Starting the installer".into();
                local.note = "SuperChat closes now so the installer can replace it. It starts again when the installer finishes.".into();
                local.action = String::new();
                slint::Timer::single_shot(Duration::from_millis(1200), || {
                    let _ = slint::quit_event_loop();
                });
            }
            Err(e) => local.error = Some(format!("Could not start the installer: {e}. It is saved at {}.", file.display())),
        },
        Platform::WindowsPortable => {
            local.title = format!("Version {v} is downloaded");
            local.note = format!("Saved as {}. Close SuperChat and run that file to use the new version; the old file can then be deleted.", file.display());
            local.action = "Show the file".into();
        }
        Platform::Flatpak | Platform::Other => {
            local.title = "Downloaded".into();
            local.note = format!("Saved as {}.", file.display());
        }
    }
    let _ = ui;
}

/// "Restart now": starts the new AppImage a moment after this process has ended and quits.
fn restart(target: &PathBuf) {
    let _ = std::process::Command::new("sh").arg("-c").arg("sleep 1; exec \"$0\"").arg(target).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    let _ = slint::quit_event_loop();
}

/// Creates the updater, wires the callbacks and starts the refresh/auto-check timer. Keep the returned timer alive.
pub fn setup(ui: &AppWindow, hooks: &Arc<Hooks>, sh: &Sh) -> slint::Timer {
    let (url, auto) = {
        let g = sh.lock().unwrap();
        (update::update_url(&g.prefs.update_url), g.prefs.auto_update_check)
    };
    let platform = update::detect_platform();
    let up = Updater::new(&hooks.data_dir, crate::runtime().handle().clone(), client_core::CLIENT_VERSION, platform, url);
    // Downloads of finished or older installs are not needed any more.
    let dl_dir = Updater::download_dir(platform, &hooks.data_dir);
    if up.status().phase != Phase::Available {
        let _ = std::fs::remove_dir_all(hooks.data_dir.join("updates"));
    }
    let local = Arc::new(Mutex::new(Local::default()));
    let _ = auto;
    ui.set_version_long(version_long(hooks.version_code.as_ref().and_then(|f| f())).into());

    let download = {
        let (up, dl_dir) = (up.clone(), dl_dir.clone());
        move || up.download(dl_dir.clone())
    };
    {
        let up = up.clone();
        ui.on_upd_check(move || up.check());
    }
    {
        let (wk, up, local, hooks, dl) = (ui.as_weak(), up.clone(), local.clone(), hooks.clone(), download.clone());
        let dl_dir = dl_dir.clone();
        ui.on_upd_primary_clicked(move || {
            let Some(ui) = wk.upgrade() else { return };
            let st = up.status();
            match st.phase {
                Phase::Available => {
                    if st.pick.is_some() {
                        *local.lock().unwrap() = Local::default();
                        dl();
                    } else if let Some(r) = &st.release {
                        let url = if r.html_url.is_empty() { update::RELEASES_PAGE.to_string() } else { r.html_url.clone() };
                        if !open_page(&hooks, &url) {
                            toast(&ui, &format!("No web browser found. The download page is {url}"));
                        }
                    }
                }
                Phase::Failed => {
                    *local.lock().unwrap() = Local::default();
                    up.retry(dl_dir.clone());
                }
                Phase::Ready => {
                    let mut l = local.lock().unwrap();
                    if let Some(t) = l.restart.clone() {
                        drop(l);
                        restart(&t);
                    } else if st.platform == Platform::WindowsPortable {
                        if let Some(f) = st.file.as_ref().and_then(|f| f.parent()) {
                            platform::open_path(f);
                        }
                    } else {
                        install(&ui, &hooks, &st, &mut l);
                    }
                }
                _ => {}
            }
        });
    }
    {
        let up = up.clone();
        ui.on_upd_cancel(move || up.cancel());
    }
    {
        let (wk, up) = (ui.as_weak(), up.clone());
        ui.on_upd_skip(move || {
            up.skip_version();
            if let Some(ui) = wk.upgrade() {
                toast(&ui, "This version will not be offered again. A newer one will.");
            }
        });
    }
    {
        let up = up.clone();
        ui.on_upd_remind(move || up.remind_later(now_ms()));
    }
    {
        let (wk, local) = (ui.as_weak(), local.clone());
        ui.on_upd_copy_command(move || {
            let Some(ui) = wk.upgrade() else { return };
            let cmd = local.lock().unwrap().command.clone();
            toast(&ui, if platform::copy_text(&cmd) { "Command copied." } else { "Could not copy. Select the text instead." });
        });
    }
    {
        let sh = sh.clone();
        ui.on_upd_auto_toggled(move |on| {
            let mut g = sh.lock().unwrap();
            g.prefs.auto_update_check = on;
            crate::save_prefs(&g);
        });
    }
    {
        let wk = ui.as_weak();
        ui.on_open_about(move || {
            let Some(ui) = wk.upgrade() else { return };
            ui.set_menu_kind("".into());
            ui.invoke_open_settings();
            ui.set_settings_page("about".into());
        });
    }

    let timer = slint::Timer::default();
    {
        let (wk, up, local, hooks, sh) = (ui.as_weak(), up, local, hooks.clone(), sh.clone());
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(300), move || {
            let Some(ui) = wk.upgrade() else { return };
            let now = now_ms();
            ui.set_version_long(version_long(hooks.version_code.as_ref().and_then(|f| f())).into());
            let auto = sh.lock().unwrap().prefs.auto_update_check;
            up.auto_tick(auto, now);
            let st = up.status();
            let mut l = local.lock().unwrap();
            // Android's installer talks back asynchronously.
            if let Some((kind, msg)) = (hooks.take_install_result)() {
                match kind.as_str() {
                    "cancelled" => {
                        l.title = "Installation cancelled".into();
                        l.note = "Nothing was changed. Tap Install again when you are ready.".into();
                        l.action = "Install again".into();
                    }
                    "permission" => {
                        l.title = "Allow installs from SuperChat".into();
                        l.note = "Android needs your permission first. Turn on \"Allow from this source\" on the settings page that opened, then press Back: the install continues by itself.".into();
                        l.action = "Install again".into();
                    }
                    _ => l.error = Some(msg),
                }
            }
            if st.phase == Phase::Ready && l.handled.as_ref() != st.file.as_ref() {
                install(&ui, &hooks, &st, &mut l);
            }
            if st.phase != Phase::Ready && l.handled.is_some() {
                *l = Local::default();
            }
            show(&ui, &st, &l, auto, now);
        });
    }
    timer
}

/// Opens a web page; false when the platform has no browser to do it.
fn open_page(hooks: &Hooks, url: &str) -> bool {
    match &hooks.open_url {
        Some(f) => f(url),
        None => {
            platform::open_url(url);
            true
        }
    }
}

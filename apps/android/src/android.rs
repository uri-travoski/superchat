//! Android glue: the native activity entry (`android_main`) and JNI hooks used by the Kotlin
//! shell (foreground service, notification taps, system font scale, time zone, share target, media,
//! Keystore-backed secrets and local notifications).

use atomic_ui::{Media, Shared};
use client_core::error::{ClientError, Result as CcResult};
use client_core::secrets::{FileSecretStore, SecretStore};
use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

static VM: OnceLock<JavaVM> = OnceLock::new();
static NATIVE: OnceLock<GlobalRef> = OnceLock::new();
static SHARED: Mutex<Option<Shared>> = Mutex::new(None);
static LOGIN_LINK: Mutex<Option<String>> = Mutex::new(None);

/// Runs `f` on a JNI-attached thread with the `app.superchat.Native` class. None when the shell is not ready or Java threw.
fn with_env<R>(f: impl FnOnce(&mut JNIEnv, &JClass) -> Option<R>) -> Option<R> {
    let vm = VM.get()?;
    let cls = NATIVE.get()?;
    let mut env = vm.attach_current_thread_permanently().ok()?;
    let jc = unsafe { JClass::from_raw(cls.as_obj().as_raw()) };
    let r = f(&mut env, &jc);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
        return None;
    }
    r
}

fn call_bool(name: &str) -> bool {
    with_env(|env, c| env.call_static_method(c, name, "()Z", &[]).ok()?.z().ok()).unwrap_or(false)
}

fn call_string(name: &str, arg: &str) -> Option<String> {
    with_env(|env, c| {
        let a = env.new_string(arg).ok()?;
        let o = env.call_static_method(c, name, "(Ljava/lang/String;)Ljava/lang/String;", &[JValue::Object(&a)]).ok()?.l().ok()?;
        if o.is_null() {
            return None;
        }
        let js: JString = o.into();
        let s: String = env.get_string(&js).ok()?.into();
        Some(s)
    })
}

/// Audio playback and MediaRecorder recording in the Kotlin shell.
struct AndroidMedia;

impl Media for AndroidMedia {
    fn play(&self, path: &Path) -> bool {
        call_string("play", &path.to_string_lossy()).map(|s| s == "ok").unwrap_or(false)
    }
    fn stop(&self) {
        with_env(|env, c| env.call_static_method(c, "stop", "()V", &[]).ok().map(|_| ()));
    }
    fn playing(&self) -> bool {
        call_bool("isPlaying")
    }
    fn can_record(&self) -> bool {
        true
    }
    fn record_start(&self) -> bool {
        call_bool("recordStart")
    }
    fn record_stop(&self) -> Option<PathBuf> {
        let s = with_env(|env, c| {
            let o = env.call_static_method(c, "recordStop", "()Ljava/lang/String;", &[]).ok()?.l().ok()?;
            if o.is_null() {
                return None;
            }
            let js: JString = o.into();
            let s: String = env.get_string(&js).ok()?.into();
            Some(s)
        })?;
        if s.is_empty() { None } else { Some(PathBuf::from(s)) }
    }
}

/// X-02: values are encrypted with an AES-GCM key that lives in the Android Keystore (it never leaves the secure
/// hardware / keystore process); only ciphertext is written to the app's private files. Older builds stored tokens in a
/// plain 0600 file: those are migrated on first read and the file is deleted.
pub struct AndroidSecrets {
    path: PathBuf,
    old: FileSecretStore,
    lock: Mutex<()>,
}

impl AndroidSecrets {
    pub fn new(dir: &Path) -> AndroidSecrets {
        AndroidSecrets { path: dir.join("client").join("secrets.keystore.json"), old: FileSecretStore::new(dir.join("client").join("secrets.json")), lock: Mutex::new(()) }
    }
    fn read(&self) -> std::collections::BTreeMap<String, String> {
        std::fs::read_to_string(&self.path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    fn write(&self, m: &std::collections::BTreeMap<String, String>) -> CcResult<()> {
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&self.path, serde_json::to_vec(m)?)?;
        Ok(())
    }
}

impl SecretStore for AndroidSecrets {
    fn get(&self, key: &str) -> CcResult<Option<String>> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ct) = self.read().get(key) {
            return Ok(call_string("decrypt", ct));
        }
        if let Some(v) = self.old.get(key)? {
            // Migrate from the plain file.
            drop(_g);
            self.set(key, &v)?;
            self.old.delete(key)?;
            return Ok(Some(v));
        }
        Ok(None)
    }
    fn set(&self, key: &str, value: &str) -> CcResult<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let ct = call_string("encrypt", value).ok_or_else(|| ClientError::Local("the Android Keystore refused the secret".into()))?;
        let mut m = self.read();
        m.insert(key.to_string(), ct);
        self.write(&m)
    }
    fn delete(&self, key: &str) -> CcResult<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut m = self.read();
        m.remove(key);
        self.write(&m)?;
        self.old.delete(key)
    }
}

/// Whichever of the Activity or the service starts first opens the shared client; both must pass the same store.
fn client_for(dir: &Path) -> client_core::Client {
    atomic_ui::shared_client_with(dir, insecure_allowed(dir), "Android", Some(Arc::new(AndroidSecrets::new(dir))))
}

static OPEN_CONV: Mutex<Option<String>> = Mutex::new(None);
static FONT_SCALE_BITS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000); // 1.0

/// Plain http is refused unless this marker file exists in the app's private files dir
/// (used only for emulator tests against a dev server: `run-as app.superchat touch files/allow_http`).
fn insecure_allowed(dir: &std::path::Path) -> bool {
    dir.join("allow_http").exists()
}

#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("atomic"));
    std::panic::set_hook(Box::new(|i| log::error!("panic: {i}")));
    let dir: PathBuf = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
    let dir2 = dir.clone();
    slint::android::init(app).expect("slint android init");
    let hooks = atomic_ui::Hooks {
        allow_insecure_http: insecure_allowed(&dir),
        data_dir: dir,
        device_name: "Android".into(),
        system_scale: Box::new(|| f32::from_bits(FONT_SCALE_BITS.load(std::sync::atomic::Ordering::Relaxed))),
        take_open_conv: Box::new(|| OPEN_CONV.lock().ok().and_then(|mut g| g.take())),
        secrets: Some(Arc::new(AndroidSecrets::new(&dir2))),
        is_android: true,
        media: Arc::new(AndroidMedia),
        take_shared: Box::new(|| SHARED.lock().ok().and_then(|mut g| g.take())),
        scan_qr: Some(Box::new(|| {
            with_env(|env, c| env.call_static_method(c, "scanQr", "()V", &[]).ok().map(|_| ()));
        })),
        take_login_link: Box::new(|| LOGIN_LINK.lock().ok().and_then(|mut g| g.take())),
        clipboard: Some(Box::new(|| call_string("clipboardText", "").filter(|s| !s.is_empty()))),
    };
    if let Err(e) = atomic_ui::run_with(hooks) {
        log::error!("ui error: {e}");
    }
}

fn jstr(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(|s| s.into()).unwrap_or_default()
}

/// Called by `Native.init`: remembers the VM and the helper class so Rust can call back into Kotlin.
#[no_mangle]
pub extern "system" fn Java_app_superchat_Native_init(env: JNIEnv, class: JClass) {
    if let Ok(vm) = env.get_java_vm() {
        let _ = VM.set(vm);
    }
    if let Ok(g) = env.new_global_ref(JObject::from(class)) {
        let _ = NATIVE.set(g);
    }
}

/// Called by the Kotlin foreground service: keeps the realtime connection alive outside the Activity and shows
/// local notifications for incoming messages (N-01).
#[no_mangle]
pub extern "system" fn Java_app_superchat_SyncService_nativeStart(mut env: JNIEnv, _c: JClass, data_dir: JString) {
    let dir = PathBuf::from(jstr(&mut env, &data_dir));
    let client = client_for(&dir);
    if client.signed_in() {
        client.start();
    }
    static NOTIFIER: std::sync::Once = std::sync::Once::new();
    NOTIFIER.call_once(|| {
        let c = client.clone();
        let mut rx = client.subscribe();
        client.handle().spawn(async move {
            while let Ok(ev) = rx.recv().await {
                if let client_core::ClientEvent::Incoming { message, conversation, sender_name } = ev {
                    let prefs = client_core::Prefs::load(&dir);
                    let me = c.me();
                    let pref_name = me.as_ref().map(|m| m.presence_pref.clone()).unwrap_or_else(|| "auto".into());
                    let my_id = c.my_id().unwrap_or_default();
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
                    let allowed = proto::rules::should_notify(Some(&c.pref(&message.conv_id)), now, &pref_name, message.mentions.contains(&my_id));
                    if !prefs.notifications || pref_name == "dnd" || !allowed {
                        continue;
                    }
                    let title = conversation.as_ref().map(|cs| c.conversation_title(cs)).unwrap_or_else(|| sender_name.clone());
                    let body = if prefs.notification_preview { format!("{}: {}", sender_name, client_core::logic::markup::to_plain(&message.body)) } else { "New message".to_string() };
                    with_env(|env, c| {
                        let t = env.new_string(&title).ok()?;
                        let b = env.new_string(&body).ok()?;
                        let cv = env.new_string(&message.conv_id).ok()?;
                        env.call_static_method(c, "notifyMessage", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V", &[JValue::Object(&t), JValue::Object(&b), JValue::Object(&cv)]).ok().map(|_| ())
                    });
                }
            }
        });
    });
}

/// A push (ntfy or FCM) arrived: sync now. Pushes carry no message text.
#[no_mangle]
pub extern "system" fn Java_app_superchat_Native_nativeWake(mut env: JNIEnv, _c: JClass, data_dir: JString) {
    let dir = PathBuf::from(jstr(&mut env, &data_dir));
    let client = client_for(&dir);
    if client.signed_in() {
        client.start();
        client.reconnect_now();
    }
}

/// The Kotlin shell pushes the local UTC offset (chrono cannot read the zone on Android).
#[no_mangle]
pub extern "system" fn Java_app_superchat_Native_nativeTzOffset(_env: JNIEnv, _c: JClass, secs: i32) {
    atomic_ui::set_tz_offset(secs);
}

/// Text and file paths received through the share sheet (E-06); files are newline separated.
#[no_mangle]
pub extern "system" fn Java_app_superchat_Native_nativeShare(mut env: JNIEnv, _c: JClass, text: JString, files: JString) {
    let text = jstr(&mut env, &text);
    let files: Vec<PathBuf> = jstr(&mut env, &files).lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect();
    if let Ok(mut g) = SHARED.lock() {
        *g = Some((text, files));
    }
}

/// A one-time login link from the QR scanner or a tapped superchat:// link.
#[no_mangle]
pub extern "system" fn Java_app_superchat_Native_nativeLoginLink(mut env: JNIEnv, _c: JClass, link: JString) {
    let link = jstr(&mut env, &link);
    if let Ok(mut g) = LOGIN_LINK.lock() {
        *g = Some(link);
    }
}

/// Called by MainActivity when a notification is tapped.
#[no_mangle]
pub extern "system" fn Java_app_superchat_Native_nativeOpenConversation(mut env: JNIEnv, _c: JClass, conv: JString) {
    let conv = jstr(&mut env, &conv);
    log::info!("open conversation {conv}");
    if let Ok(mut g) = OPEN_CONV.lock() {
        *g = Some(conv);
    }
}

/// Android system font scale (Settings > Display > Font size), pushed by MainActivity.
#[no_mangle]
pub extern "system" fn Java_app_superchat_Native_nativeFontScale(_env: JNIEnv, _c: JClass, scale: f32) {
    FONT_SCALE_BITS.store(scale.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// android-activity's bundled C++ (games-sdk glue) is compiled against NDK 27's libc++ headers, which
/// call `std::__ndk1::__hash_memory`. Emulator/device images older than that libc++ do not export it,
/// so the library would fail to load ("cannot locate symbol"). Provide it here (FNV-1a; the value only
/// needs to be a consistent hash for unordered containers).
#[no_mangle]
pub unsafe extern "C" fn _ZNSt6__ndk113__hash_memoryEPKvm(p: *const u8, n: usize) -> usize {
    let mut h: u64 = 0xcbf29ce484222325;
    for i in 0..n {
        h ^= *p.add(i) as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h as usize
}

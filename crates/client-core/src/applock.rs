//! App Lock: a local 6-digit PIN that hides chosen conversations.
//!
//! * The PIN never leaves the device. Only a salted Argon2id hash is stored, inside the platform secret store
//!   (libsecret / Credential Manager / Android Keystore) together with the ids of the locked conversations and the
//!   wrong-attempt counter, so none of it appears in plain files.
//! * Locked is the default state: with a PIN set the app starts locked, re-locks after five minutes without activity,
//!   after five minutes in the background, and on request.
//! * Wrong PINs are rate limited: five in a row block entry for 30 s, every further wrong PIN doubles the wait (up to
//!   one hour). The counter survives restarts.
//! * Honest limit: this hides chats in the app, it does not encrypt the local cache. Someone with the unlocked device and
//!   file access could read the cache; the PIN has only a million values, so the delay protects the screen, not the disk.
//!
//! Every time-dependent function takes `now_ms` so the logic is testable without sleeping.

use crate::error::{ClientError, Result};
use crate::secrets::SecretStore;
use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

pub const SECRET_KEY: &str = "superchat.applock";
pub const PIN_LEN: usize = 6;
/// Idle time (and time in the background) after which the app locks again.
pub const IDLE_TIMEOUT_MS: i64 = 5 * 60 * 1000;
/// Wrong PINs in a row before the first delay.
pub const FREE_ATTEMPTS: u32 = 5;
pub const FIRST_DELAY_SECS: u64 = 30;
pub const MAX_DELAY_SECS: u64 = 3600;

pub fn wall_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unlock {
    Ok,
    Wrong,
    /// Too many wrong PINs: try again in this many seconds (the PIN is not even checked).
    Blocked(u64),
    NotSet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PinError {
    /// Not exactly six digits.
    Invalid,
    /// The two entries differ.
    Mismatch,
    Wrong,
    Blocked(u64),
    NotSet,
    AlreadySet,
}

impl std::fmt::Display for PinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PinError::Invalid => write!(f, "The PIN must be exactly 6 digits."),
            PinError::Mismatch => write!(f, "The two PINs do not match."),
            PinError::Wrong => write!(f, "Incorrect PIN"),
            PinError::Blocked(s) => write!(f, "Too many attempts. Try again in {s} seconds."),
            PinError::NotSet => write!(f, "No PIN is set."),
            PinError::AlreadySet => write!(f, "A PIN is already set."),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct Record {
    salt: String,
    hash: String,
    m_kib: u32,
    t: u32,
    fails: u32,
    blocked_until_ms: i64,
    locked: BTreeSet<String>,
}

#[derive(Default)]
struct State {
    rec: Option<Record>,
    engaged: bool,
    last_activity_ms: i64,
    background_since_ms: Option<i64>,
}

/// Argon2id cost. `Cost::default()` is for real use; tests pass something tiny.
#[derive(Clone, Copy, Debug)]
pub struct Cost {
    pub m_kib: u32,
    pub t: u32,
}

impl Default for Cost {
    fn default() -> Self {
        Cost { m_kib: 19 * 1024, t: 2 }
    }
}

impl Cost {
    pub fn tiny() -> Cost {
        Cost { m_kib: 8, t: 1 }
    }
}

pub struct AppLock {
    state: Mutex<State>,
    store: Arc<dyn SecretStore>,
    cost: Cost,
}

fn derive(pin: &str, salt: &[u8], m_kib: u32, t: u32) -> Result<Vec<u8>> {
    let params = Params::new(m_kib, t, 1, Some(32)).map_err(|e| ClientError::Local(format!("pin hash: {e}")))?;
    let mut out = vec![0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params).hash_password_into(pin.as_bytes(), salt, &mut out).map_err(|e| ClientError::Local(format!("pin hash: {e}")))?;
    Ok(out)
}

fn valid_pin(p: &str) -> bool {
    p.len() == PIN_LEN && p.bytes().all(|b| b.is_ascii_digit())
}

/// Constant-time comparison of two equal-length byte strings.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Seconds of waiting after `fails` wrong PINs in a row (0 while attempts are free).
pub fn delay_secs(fails: u32) -> u64 {
    if fails < FREE_ATTEMPTS {
        return 0;
    }
    let doublings = (fails - FREE_ATTEMPTS).min(20);
    (FIRST_DELAY_SECS.saturating_mul(1u64 << doublings)).min(MAX_DELAY_SECS)
}

impl AppLock {
    pub fn open(store: Arc<dyn SecretStore>) -> AppLock {
        Self::open_with(store, Cost::default())
    }

    pub fn open_with(store: Arc<dyn SecretStore>, cost: Cost) -> AppLock {
        let rec: Option<Record> = store.get(SECRET_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok());
        // Locked is the default: a PIN means the app starts locked.
        let engaged = rec.is_some();
        AppLock { state: Mutex::new(State { rec, engaged, last_activity_ms: wall_ms(), background_since_ms: None }), store, cost }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, s: &State) -> Result<()> {
        match &s.rec {
            Some(r) => self.store.set(SECRET_KEY, &serde_json::to_string(r)?),
            None => self.store.delete(SECRET_KEY),
        }
    }

    pub fn is_set(&self) -> bool {
        self.lock_state().rec.is_some()
    }

    /// True while locked chats are hidden.
    pub fn is_engaged(&self) -> bool {
        let s = self.lock_state();
        s.rec.is_some() && s.engaged
    }

    /// Seconds until PIN entry is allowed again (0 = now).
    pub fn blocked_secs(&self, now_ms: i64) -> u64 {
        let s = self.lock_state();
        match &s.rec {
            Some(r) if r.blocked_until_ms > now_ms => ((r.blocked_until_ms - now_ms + 999) / 1000) as u64,
            _ => 0,
        }
    }

    // ------------------------------------------------------------------ PIN management

    /// Sets the first PIN (entered twice). The app stays unlocked afterwards: the person just proved they own it.
    pub fn set_pin(&self, pin: &str, again: &str, now_ms: i64) -> std::result::Result<(), PinError> {
        if !valid_pin(pin) {
            return Err(PinError::Invalid);
        }
        if pin != again {
            return Err(PinError::Mismatch);
        }
        let mut s = self.lock_state();
        if s.rec.is_some() {
            return Err(PinError::AlreadySet);
        }
        s.rec = Some(self.new_record(pin, BTreeSet::new()).map_err(|_| PinError::Invalid)?);
        s.engaged = false;
        s.last_activity_ms = now_ms;
        self.save(&s).map_err(|_| PinError::Invalid)?;
        Ok(())
    }

    fn new_record(&self, pin: &str, locked: BTreeSet<String>) -> Result<Record> {
        use rand::RngCore;
        let mut salt = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut salt);
        let hash = derive(pin, &salt, self.cost.m_kib, self.cost.t)?;
        Ok(Record { salt: hex::encode(salt), hash: hex::encode(hash), m_kib: self.cost.m_kib, t: self.cost.t, fails: 0, blocked_until_ms: 0, locked })
    }

    /// Checks `pin` against the stored hash with rate limiting. Counts failures and persists the counter.
    fn check(&self, s: &mut State, pin: &str, now_ms: i64) -> Unlock {
        let Some(rec) = s.rec.as_mut() else { return Unlock::NotSet };
        if rec.blocked_until_ms > now_ms {
            return Unlock::Blocked(((rec.blocked_until_ms - now_ms + 999) / 1000) as u64);
        }
        let ok = valid_pin(pin)
            && hex::decode(&rec.salt).ok().and_then(|salt| derive(pin, &salt, rec.m_kib, rec.t).ok()).map(|h| ct_eq(&h, &hex::decode(&rec.hash).unwrap_or_default())).unwrap_or(false);
        if ok {
            if rec.fails != 0 || rec.blocked_until_ms != 0 {
                rec.fails = 0;
                rec.blocked_until_ms = 0;
                let _ = self.save(s);
            }
            return Unlock::Ok;
        }
        rec.fails = rec.fails.saturating_add(1);
        let d = delay_secs(rec.fails);
        if d > 0 {
            rec.blocked_until_ms = now_ms + (d as i64) * 1000;
        }
        let _ = self.save(s);
        Unlock::Wrong
    }

    /// Correct PIN: locked chats become visible until the idle timeout. Wrong PIN: counts toward the delay.
    pub fn unlock(&self, pin: &str, now_ms: i64) -> Unlock {
        let mut s = self.lock_state();
        let r = self.check(&mut s, pin, now_ms);
        if r == Unlock::Ok {
            s.engaged = false;
            s.last_activity_ms = now_ms;
            s.background_since_ms = None;
        }
        r
    }

    /// Changes the PIN; needs the current one.
    pub fn change_pin(&self, current: &str, new: &str, again: &str, now_ms: i64) -> std::result::Result<(), PinError> {
        if !valid_pin(new) {
            return Err(PinError::Invalid);
        }
        if new != again {
            return Err(PinError::Mismatch);
        }
        let mut s = self.lock_state();
        match self.check(&mut s, current, now_ms) {
            Unlock::Ok => {}
            Unlock::Wrong => return Err(PinError::Wrong),
            Unlock::Blocked(n) => return Err(PinError::Blocked(n)),
            Unlock::NotSet => return Err(PinError::NotSet),
        }
        let locked = s.rec.as_ref().map(|r| r.locked.clone()).unwrap_or_default();
        s.rec = Some(self.new_record(new, locked).map_err(|_| PinError::Invalid)?);
        self.save(&s).map_err(|_| PinError::Invalid)?;
        Ok(())
    }

    /// Removes the PIN (needs it). Every locked chat becomes an ordinary chat again.
    pub fn remove_pin(&self, current: &str, now_ms: i64) -> std::result::Result<(), PinError> {
        let mut s = self.lock_state();
        match self.check(&mut s, current, now_ms) {
            Unlock::Ok => {}
            Unlock::Wrong => return Err(PinError::Wrong),
            Unlock::Blocked(n) => return Err(PinError::Blocked(n)),
            Unlock::NotSet => return Err(PinError::NotSet),
        }
        s.rec = None;
        s.engaged = false;
        self.save(&s).map_err(|_| PinError::Invalid)?;
        Ok(())
    }

    /// Forgets everything (sign-out or forced wipe: the lock belongs to the account's local data).
    pub fn clear(&self) {
        let mut s = self.lock_state();
        s.rec = None;
        s.engaged = false;
        let _ = self.save(&s);
    }

    // ------------------------------------------------------------------ lock / idle logic

    /// Locks now (the padlock was tapped).
    pub fn lock(&self) {
        let mut s = self.lock_state();
        if s.rec.is_some() {
            s.engaged = true;
        }
    }

    /// Any tap or key press while unlocked postpones the idle lock.
    pub fn activity(&self, now_ms: i64) {
        let mut s = self.lock_state();
        if !s.engaged {
            s.last_activity_ms = now_ms;
        }
    }

    /// Locks when nothing happened for the idle timeout. Returns true when this call locked the app.
    pub fn check_idle(&self, now_ms: i64) -> bool {
        let mut s = self.lock_state();
        if s.rec.is_some() && !s.engaged && now_ms - s.last_activity_ms >= IDLE_TIMEOUT_MS {
            s.engaged = true;
            return true;
        }
        false
    }

    /// The app went to the background (`true`) or came back (`false`). Returning after the timeout locks at once.
    /// Returns true when this call locked the app.
    pub fn set_background(&self, background: bool, now_ms: i64) -> bool {
        let mut s = self.lock_state();
        if background {
            if s.background_since_ms.is_none() {
                s.background_since_ms = Some(now_ms);
            }
            return false;
        }
        let since = s.background_since_ms.take();
        let away_long = since.map(|t| now_ms - t >= IDLE_TIMEOUT_MS).unwrap_or(false);
        let idle_long = now_ms - s.last_activity_ms >= IDLE_TIMEOUT_MS;
        if s.rec.is_some() && !s.engaged && (away_long || idle_long) {
            s.engaged = true;
            return true;
        }
        if !s.engaged {
            s.last_activity_ms = now_ms;
        }
        false
    }

    // ------------------------------------------------------------------ locked conversations

    /// Marks or unmarks a conversation. Only possible while a PIN is set.
    pub fn set_conv_locked(&self, conv_id: &str, locked: bool) -> std::result::Result<(), PinError> {
        let mut s = self.lock_state();
        let Some(rec) = s.rec.as_mut() else { return Err(PinError::NotSet) };
        if locked {
            rec.locked.insert(conv_id.to_string());
        } else {
            rec.locked.remove(conv_id);
        }
        self.save(&s).map_err(|_| PinError::Invalid)
    }

    /// Marked as locked (whether or not the app is unlocked right now). Notifications use this.
    pub fn is_conv_locked(&self, conv_id: &str) -> bool {
        self.lock_state().rec.as_ref().map(|r| r.locked.contains(conv_id)).unwrap_or(false)
    }

    /// Must be left out of every list, search and count right now.
    pub fn is_hidden(&self, conv_id: &str) -> bool {
        let s = self.lock_state();
        s.engaged && s.rec.as_ref().map(|r| r.locked.contains(conv_id)).unwrap_or(false)
    }

    pub fn locked_ids(&self) -> Vec<String> {
        self.lock_state().rec.as_ref().map(|r| r.locked.iter().cloned().collect()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::FileSecretStore;

    fn lock() -> (AppLock, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        let l = AppLock::open_with(Arc::new(FileSecretStore::new(d.path().join("s.json"))), Cost::tiny());
        (l, d)
    }
    fn reopen(d: &tempfile::TempDir) -> AppLock {
        AppLock::open_with(Arc::new(FileSecretStore::new(d.path().join("s.json"))), Cost::tiny())
    }
    const T0: i64 = 1_000_000;

    #[test]
    fn pin_must_be_six_digits_entered_twice() {
        let (l, _d) = lock();
        assert_eq!(l.set_pin("12345", "12345", T0), Err(PinError::Invalid));
        assert_eq!(l.set_pin("12345a", "12345a", T0), Err(PinError::Invalid));
        assert_eq!(l.set_pin("1234567", "1234567", T0), Err(PinError::Invalid));
        assert_eq!(l.set_pin("123456", "123457", T0), Err(PinError::Mismatch));
        assert!(!l.is_set());
        l.set_pin("123456", "123456", T0).unwrap();
        assert!(l.is_set());
        assert!(!l.is_engaged(), "just set: stays unlocked");
        assert_eq!(l.set_pin("654321", "654321", T0), Err(PinError::AlreadySet));
    }

    #[test]
    fn only_a_salted_hash_is_stored() {
        let (l, d) = lock();
        l.set_pin("246810", "246810", T0).unwrap();
        let raw = std::fs::read_to_string(d.path().join("s.json")).unwrap();
        assert!(!raw.contains("246810"));
        let (l2, d2) = lock();
        l2.set_pin("246810", "246810", T0).unwrap();
        let raw2 = std::fs::read_to_string(d2.path().join("s.json")).unwrap();
        let hash = |r: &str| serde_json::from_str::<serde_json::Value>(&serde_json::from_str::<std::collections::BTreeMap<String, String>>(r).unwrap()[SECRET_KEY]).unwrap()["hash"].to_string();
        assert_ne!(hash(&raw), hash(&raw2), "a fresh salt per PIN: equal PINs give different hashes");
    }

    #[test]
    fn locked_by_default_after_restart_and_unlock_needs_the_pin() {
        let (l, d) = lock();
        l.set_pin("111222", "111222", T0).unwrap();
        l.set_conv_locked("c1", true).unwrap();
        assert!(!l.is_hidden("c1"), "unlocked right after setting the PIN");
        let l2 = reopen(&d);
        assert!(l2.is_engaged(), "the app starts locked");
        assert!(l2.is_hidden("c1") && !l2.is_hidden("other"));
        assert_eq!(l2.unlock("000000", T0), Unlock::Wrong);
        assert!(l2.is_hidden("c1"));
        assert_eq!(l2.unlock("111222", T0), Unlock::Ok);
        assert!(!l2.is_hidden("c1") && l2.is_conv_locked("c1"));
    }

    #[test]
    fn five_wrong_pins_block_for_30s_then_doubling() {
        assert_eq!([4, 5, 6, 7, 8, 30].map(delay_secs), [0, 30, 60, 120, 240, 3600]);
        let (l, _d) = lock();
        l.set_pin("111111", "111111", T0).unwrap();
        l.lock();
        for i in 0..5 {
            assert_eq!(l.unlock("999999", T0 + i), Unlock::Wrong);
        }
        // Blocked: even the right PIN is refused during the delay and does not count.
        match l.unlock("111111", T0 + 10_000) {
            Unlock::Blocked(s) => assert!((20..=21).contains(&s), "{s}"),
            o => panic!("{o:?}"),
        }
        assert!(l.is_engaged());
        assert_eq!(l.blocked_secs(T0 + 10_000), 21);
        // After 30 s one more wrong PIN doubles the wait to 60 s.
        let t = T0 + 31_000;
        assert_eq!(l.unlock("999999", t), Unlock::Wrong);
        assert_eq!(l.blocked_secs(t), 60);
        assert!(matches!(l.unlock("111111", t + 59_000), Unlock::Blocked(_)));
        assert_eq!(l.unlock("111111", t + 60_001), Unlock::Ok, "the right PIN works once the delay is over and resets the counter");
        l.lock();
        assert_eq!(l.unlock("999999", t + 61_000), Unlock::Wrong);
        assert_eq!(l.blocked_secs(t + 61_000), 0, "the counter restarted");
    }

    #[test]
    fn the_delay_survives_a_restart() {
        let (l, d) = lock();
        l.set_pin("111111", "111111", T0).unwrap();
        for i in 0..5 {
            l.unlock("999999", T0 + i);
        }
        let l2 = reopen(&d);
        assert!(l2.blocked_secs(T0 + 1000) > 0);
        assert!(matches!(l2.unlock("111111", T0 + 1000), Unlock::Blocked(_)));
    }

    #[test]
    fn idle_timeout_is_five_minutes_and_activity_postpones_it() {
        let (l, _d) = lock();
        l.set_pin("111111", "111111", T0).unwrap();
        assert!(!l.check_idle(T0 + IDLE_TIMEOUT_MS - 1));
        l.activity(T0 + IDLE_TIMEOUT_MS - 1);
        assert!(!l.check_idle(T0 + IDLE_TIMEOUT_MS + 1000), "a tap restarted the clock");
        assert!(l.check_idle(T0 + 2 * IDLE_TIMEOUT_MS), "locked after 5 idle minutes");
        assert!(l.is_engaged());
        l.activity(T0 + 3 * IDLE_TIMEOUT_MS);
        assert!(l.is_engaged(), "activity does not unlock");
        assert_eq!(l.unlock("111111", T0 + 3 * IDLE_TIMEOUT_MS), Unlock::Ok);
        assert!(!l.check_idle(T0 + 3 * IDLE_TIMEOUT_MS + 1000));
    }

    #[test]
    fn background_for_five_minutes_locks_on_return() {
        let (l, _d) = lock();
        l.set_pin("111111", "111111", T0).unwrap();
        assert!(!l.set_background(true, T0));
        assert!(!l.set_background(false, T0 + IDLE_TIMEOUT_MS - 1000), "a short trip to another app keeps it open");
        assert!(!l.is_engaged());
        assert!(!l.set_background(true, T0 + IDLE_TIMEOUT_MS));
        assert!(l.set_background(false, T0 + 2 * IDLE_TIMEOUT_MS + 1000), "away more than 5 minutes: locked on return");
        assert!(l.is_engaged());
    }

    #[test]
    fn explicit_lock_and_no_pin_means_nothing_is_hidden() {
        let (l, _d) = lock();
        assert!(l.set_conv_locked("c1", true).is_err(), "marking needs a PIN");
        assert!(!l.is_engaged() && !l.is_hidden("c1"));
        l.lock();
        assert!(!l.is_engaged(), "nothing to lock without a PIN");
        l.set_pin("111111", "111111", T0).unwrap();
        l.set_conv_locked("c1", true).unwrap();
        l.lock();
        assert!(l.is_hidden("c1"));
        assert_eq!(l.unlock("111111", T0), Unlock::Ok);
        l.set_conv_locked("c1", false).unwrap();
        l.lock();
        assert!(!l.is_hidden("c1"));
    }

    #[test]
    fn change_and_remove_need_the_current_pin() {
        let (l, d) = lock();
        l.set_pin("111111", "111111", T0).unwrap();
        l.set_conv_locked("c1", true).unwrap();
        assert_eq!(l.change_pin("000000", "222222", "222222", T0), Err(PinError::Wrong));
        assert_eq!(l.change_pin("111111", "222222", "333333", T0), Err(PinError::Mismatch));
        assert_eq!(l.change_pin("111111", "22", "22", T0), Err(PinError::Invalid));
        l.change_pin("111111", "222222", "222222", T0).unwrap();
        let l2 = reopen(&d);
        assert_eq!(l2.unlock("111111", T0), Unlock::Wrong);
        assert_eq!(l2.unlock("222222", T0), Unlock::Ok);
        assert!(l2.is_conv_locked("c1"), "locked chats survive a PIN change");
        assert_eq!(l2.remove_pin("111111", T0), Err(PinError::Wrong));
        l2.remove_pin("222222", T0).unwrap();
        assert!(!l2.is_set() && !l2.is_conv_locked("c1"), "removing the PIN frees every chat");
        assert!(!reopen(&d).is_set());
    }
}

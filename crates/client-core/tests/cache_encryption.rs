//! X-04: the local cache is encrypted with SQLCipher, the key lives in the secret store, an older plain cache is
//! converted without losing anything, and a file that cannot be opened is replaced instead of crashing the app.

mod common;
use client_core::client::CACHE_KEY_NAME;
use client_core::store::{CacheState, Store};
use common::*;

const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const KEY2: &str = "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100";
const MARKER: &str = "needle-in-the-haystack-4f9c2d";

fn raw_contains(path: &std::path::Path, needle: &str) -> bool {
    // The write-ahead log can still hold the newest pages, so look in it too.
    ["", "-wal"].iter().any(|ext| std::fs::read(format!("{}{ext}", path.display())).map(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes())).unwrap_or(false))
}

#[test]
fn a_new_cache_is_encrypted_and_only_opens_with_its_key() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("cache.db");
    let (s, state) = Store::open_with_key(&p, KEY).unwrap();
    assert_eq!(state, CacheState::Created);
    assert!(s.is_encrypted());
    s.kv_set("note", MARKER).unwrap();
    drop(s);
    // Not a plain SQLite file, and the text is not readable in it.
    let head = std::fs::read(&p).unwrap();
    assert_ne!(&head[..16], b"SQLite format 3\0");
    assert!(!raw_contains(&p, MARKER));
    // Same key: the data is there.
    let (s, state) = Store::open_with_key(&p, KEY).unwrap();
    assert_eq!(state, CacheState::Opened);
    assert_eq!(s.kv_get("note").unwrap().as_deref(), Some(MARKER));
    drop(s);
    // A plain open cannot read it.
    assert!(Store::open(&p).is_err());
    // Another key: the unreadable file is kept aside and a new empty cache starts.
    let (s, state) = Store::open_with_key(&p, KEY2).unwrap();
    assert_eq!(state, CacheState::Replaced);
    assert_eq!(s.kv_get("note").unwrap(), None);
    assert!(d.path().join("cache.db.unreadable").exists());
}

#[test]
fn a_plain_cache_is_converted_without_losing_anything() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("cache.db");
    {
        let s = Store::open(&p).unwrap();
        assert!(!s.is_encrypted());
        s.kv_set("note", MARKER).unwrap();
        s.kv_set("other", "two").unwrap();
    }
    assert!(raw_contains(&p, MARKER), "the old cache is plain text on disk");
    let (s, state) = Store::open_with_key(&p, KEY).unwrap();
    assert_eq!(state, CacheState::Migrated);
    assert_eq!(s.kv_get("note").unwrap().as_deref(), Some(MARKER));
    assert_eq!(s.kv_get("other").unwrap().as_deref(), Some("two"));
    drop(s);
    assert!(!raw_contains(&p, MARKER), "nothing readable is left behind");
    assert!(!d.path().join("cache.db.enc").exists());
    // Opening again is a normal open.
    let (_s, state) = Store::open_with_key(&p, KEY).unwrap();
    assert_eq!(state, CacheState::Opened);
}

#[test]
fn wiping_the_cache_keeps_it_encrypted() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("cache.db");
    let (s, _) = Store::open_with_key(&p, KEY).unwrap();
    s.kv_set("note", MARKER).unwrap();
    s.wipe().unwrap();
    assert_eq!(s.kv_get("note").unwrap(), None);
    s.kv_set("again", MARKER).unwrap();
    drop(s);
    assert_ne!(&std::fs::read(&p).unwrap()[..16], b"SQLite format 3\0");
    assert!(!raw_contains(&p, MARKER));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_client_encrypts_its_cache_by_default_and_keeps_the_key_in_the_secret_store() {
    let s = TestServer::start().await;
    let (_id, alice) = s.user("alice").await;
    let dir = s.client_dir("alice");
    let head = std::fs::read(dir.join("cache.db")).unwrap();
    assert_ne!(&head[..16], b"SQLite format 3\0", "cache.db is encrypted");
    let secrets = std::fs::read_to_string(dir.join("secrets.json")).unwrap();
    assert!(secrets.contains(CACHE_KEY_NAME), "the key is in the secret store");
    drop(alice);
}

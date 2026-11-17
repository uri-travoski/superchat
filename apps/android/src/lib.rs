//! Android entry point. On other platforms this crate is empty so `cargo test --workspace` works.
#![cfg_attr(not(target_os = "android"), allow(unused))]

#[cfg(target_os = "android")]
mod android;

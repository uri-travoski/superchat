// Embeds the short git hash of the checkout (shown on the About page). Empty outside a git checkout.
fn main() {
    let hash = std::env::var("SUPERCHAT_COMMIT").ok().filter(|s| !s.is_empty()).or_else(|| {
        let out = std::process::Command::new("git").args(["rev-parse", "--short=7", "HEAD"]).output().ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    });
    println!("cargo:rustc-env=SUPERCHAT_COMMIT={}", hash.unwrap_or_default());
    println!("cargo:rerun-if-env-changed=SUPERCHAT_COMMIT");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
}

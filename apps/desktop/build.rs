fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/main.slint", config).expect("slint compile");
    // Windows: embed the app icon in the .exe.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        #[cfg(windows)]
        {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("../../assets/icon/superchat.ico");
            res.set("ProductName", "SuperChat");
            res.set("FileDescription", "SuperChat");
            res.compile().expect("embed windows icon");
        }
    }
}

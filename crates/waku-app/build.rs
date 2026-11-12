fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut r = winresource::WindowsResource::new();
        r.set_icon("assets/icon.ico");
        if let Err(e) = r.compile() {
            // no resource compiler (rc.exe / windres): the window still gets the logo from main.rs
            println!("cargo:warning=exe icon not embedded: {e}");
        }
    }
}

//! Windows: embed the app icon and the version information in
//! chummer-rs.exe, so Explorer, the taskbar and the installer show them.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../packaging/chummer-rs.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = env!("CARGO_PKG_VERSION");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/chummer-rs.ico")
            .set("ProductName", "chummer-rs")
            .set("FileDescription", "chummer-rs: Shadowrun 5th Edition character manager")
            .set("CompanyName", "chummer-rs")
            .set("LegalCopyright", "GPL-3.0-or-later")
            .set("OriginalFilename", "chummer-rs.exe")
            .set("ProductVersion", version)
            .set("FileVersion", version);
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon and version info: {e}");
        }
    }
}

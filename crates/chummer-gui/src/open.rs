//! Open a file or a web link with the desktop's default program.

use std::ffi::OsStr;
use std::process::{Command, Stdio};

/// Open `target` (a path or an `https://` link) the way a double-click
/// would: the default browser for links and HTML files.
pub fn open(target: impl AsRef<OsStr>) -> std::io::Result<()> {
    let target = target.as_ref();
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // Unlike `cmd /C start`, no shell parses the target (`&` in links).
        let mut c = Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler").arg(target).creation_flags(CREATE_NO_WINDOW);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(target);
        c
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(target);
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

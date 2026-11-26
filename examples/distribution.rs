//! Native, per-user installer and stable launcher. No shell/runtime dependency.
#![cfg_attr(windows, windows_subsystem = "windows")]
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
#[derive(serde::Serialize, serde::Deserialize)]
struct Entry {
    path: String,
    sha256: String,
    bytes: u64,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Current {
    format: String,
    release: String,
    version: String,
}
fn relative(s: &str) -> Result<PathBuf, String> {
    let p = Path::new(s);
    if s.is_empty()
        || s.contains(':')
        || s.contains('\0')
        || p.components().any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("unsafe package path".into());
    }
    Ok(p.into())
}
fn hash(path: &Path) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn verify(folder: &Path) -> Result<(Vec<Entry>, Vec<u8>), String> {
    let manifest = folder.join("SHA256.json");
    if fs::metadata(&manifest).map_err(|e| e.to_string())?.len() > 1_048_576 {
        return Err("oversized manifest".into());
    }
    let bytes = fs::read(manifest).map_err(|e| e.to_string())?;
    let entries: Vec<Entry> =
        serde_json::from_slice(bytes.strip_prefix(&[239, 187, 191]).unwrap_or(&bytes))
            .map_err(|e| e.to_string())?;
    if entries.is_empty() || entries.len() > 1024 {
        return Err("invalid package file count".into());
    }
    let mut paths = std::collections::BTreeSet::new();
    for e in &entries {
        let path = relative(&e.path)?;
        if !paths.insert(path.clone()) || e.bytes > 268_435_456 {
            return Err("duplicate or oversized package file".into());
        }
        let file = folder.join(path);
        let info = fs::symlink_metadata(&file).map_err(|e| e.to_string())?;
        if !info.is_file()
            || info.file_type().is_symlink()
            || info.len() != e.bytes
            || !hash(&file)?.eq_ignore_ascii_case(&e.sha256)
        {
            return Err(format!("Package verification failed: {}", e.path));
        }
        let canonical = file.canonicalize().map_err(|e| e.to_string())?;
        if !canonical.starts_with(folder.canonicalize().map_err(|e| e.to_string())?) {
            return Err("package path escapes source folder".into());
        }
    }
    if !entries.iter().any(|e| e.path == "Pelagic.exe") {
        return Err("package has no desktop app".into());
    }
    Ok((entries, bytes))
}
fn read_current(root: &Path, name: &str) -> Result<Current, String> {
    let current: Current =
        serde_json::from_slice(&fs::read(root.join(name)).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if current.format != "pelagic-install-v1"
        || relative(&current.release)?.components().count() != 1
    {
        return Err("invalid installation descriptor".into());
    }
    Ok(current)
}
fn atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    f.write_all(data)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    drop(f);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            Win32::Storage::FileSystem::{
                MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
            },
            core::PCWSTR,
        };
        let a: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: null-terminated buffers remain live throughout the call.
        unsafe {
            MoveFileExW(
                PCWSTR(a.as_ptr()),
                PCWSTR(b.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    fs::rename(temp, path).map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(windows)]
fn shortcut(root: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{System::Com::*, UI::Shell::*},
        core::{Interface, PCWSTR},
    };
    let wide = |p: &Path| {
        p.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let menu = PathBuf::from(std::env::var_os("APPDATA").ok_or("APPDATA unavailable")?)
        .join("Microsoft/Windows/Start Menu/Programs/Pelagic.lnk");
    let exe = wide(&root.join("Pelagic.exe"));
    let cwd = wide(root);
    let link = wide(&menu);
    // SAFETY: this single-threaded executable initializes COM for the lifetime of
    // its ShellLink interfaces; buffers outlive each synchronous COM call.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        let result = (|| -> windows::core::Result<()> {
            let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            shell.SetPath(PCWSTR(exe.as_ptr()))?;
            shell.SetWorkingDirectory(PCWSTR(cwd.as_ptr()))?;
            let persistent: IPersistFile = shell.cast()?;
            persistent.Save(PCWSTR(link.as_ptr()), true)?;
            Ok(())
        })();
        CoUninitialize();
        result.map_err(|e| e.to_string())
    }
}
fn run() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let source = exe.parent().ok_or("executable directory unavailable")?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--verify") {
        let folder = args
            .windows(2)
            .find(|w| w[0] == "--verify")
            .map(|w| PathBuf::from(&w[1]))
            .unwrap_or_else(|| source.into());
        verify(&folder)?;
        return Ok("Package hashes verified.".into());
    }
    let custom = args
        .windows(2)
        .find(|w| w[0] == "--destination")
        .map(|w| PathBuf::from(&w[1]));
    let installed = source.join("current.json").exists() && !args.iter().any(|a| a == "--install");
    if installed && !args.iter().any(|a| a == "--rollback") {
        let current = read_current(source, "current.json")?;
        let app = source
            .join("releases")
            .join(relative(&current.release)?)
            .join("Pelagic.exe");
        if !app.is_file() {
            return Err("Installed release is missing. Run the installer to repair it.".into());
        }
        std::process::Command::new(app)
            .current_dir(source)
            .args(&args)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(String::new());
    }
    let root = if installed {
        source.into()
    } else {
        custom.clone().unwrap_or(
            PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?)
                .join("Programs/Pelagic"),
        )
    };
    if !root.is_absolute() {
        return Err("Installation directory must be absolute.".into());
    }
    if root.exists()
        && !root.join("current.json").exists()
        && root.read_dir().map_err(|e| e.to_string())?.next().is_some()
    {
        return Err("Destination is not an empty folder or a Pelagic installation.".into());
    }
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let lock = root.join("install.lock");
    let _lock=fs::OpenOptions::new().write(true).create_new(true).open(&lock).map_err(|_|"Another installation is in progress. If it crashed, remove install.lock after closing the installer.")?;
    struct Unlock(PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _unlock = Unlock(lock);
    if args.iter().any(|a| a == "--rollback") {
        let previous = read_current(&root, "previous.json")?;
        verify(&root.join("releases").join(&previous.release))?;
        let current = fs::read(root.join("current.json")).map_err(|e| e.to_string())?;
        atomic(
            &root.join("current.json"),
            &serde_json::to_vec_pretty(&previous).unwrap(),
        )?;
        atomic(&root.join("previous.json"), &current)?;
        return Ok("Previous Pelagic release restored. Running sessions are unchanged.".into());
    }
    let (entries, bytes) = verify(source)?;
    let release = format!("{}-{:x}", env!("CARGO_PKG_VERSION"), Sha256::digest(&bytes));
    let releases = root.join("releases");
    fs::create_dir_all(&releases).map_err(|e| e.to_string())?;
    let stage = releases.join(&release);
    if stage.exists() {
        verify(&stage)?;
    } else {
        fs::create_dir(&stage).map_err(|e| e.to_string())?;
        for e in &entries {
            let rel = relative(&e.path)?;
            let dest = stage.join(&rel);
            if let Some(p) = dest.parent() {
                fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            fs::copy(source.join(rel), dest).map_err(|e| e.to_string())?;
        }
        fs::write(stage.join("SHA256.json"), &bytes).map_err(|e| e.to_string())?;
        verify(&stage)?;
    }
    let launcher = root.join("Pelagic.exe");
    if !launcher.exists() || hash(&launcher)? != hash(&exe)? {
        atomic(&launcher, &fs::read(&exe).map_err(|e| e.to_string())?)?;
    }
    // An interrupted installation never changes the selected release until all
    // payload files have passed verification. Old versions remain for rollback.
    if root.join("current.json").exists() {
        atomic(
            &root.join("previous.json"),
            &fs::read(root.join("current.json")).map_err(|e| e.to_string())?,
        )?;
    }
    let current = Current {
        format: "pelagic-install-v1".into(),
        release,
        version: env!("CARGO_PKG_VERSION").into(),
    };
    atomic(
        &root.join("current.json"),
        &serde_json::to_vec_pretty(&current).unwrap(),
    )?;
    let mut message = format!(
        "Pelagic installed at {}. Launch Pelagic.exe there, or use the Start menu. Updates keep your projects and previous release.",
        root.display()
    );
    #[cfg(windows)]
    if custom.is_none()
        && let Err(error) = shortcut(&root)
    {
        message.push_str(&format!(
            " The app is installed, but the Start menu shortcut could not be created: {error}"
        ));
    }
    Ok(message)
}
fn main() {
    let result = run();
    let quiet = std::env::args().any(|a| a == "--quiet");
    if let Some(path) = std::env::args()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|w| w[0] == "--report")
        .map(|w| w[1].clone())
    {
        let report = serde_json::json!({"ok":result.is_ok(),"message":result.as_ref().map_or_else(|e|e.clone(),Clone::clone)});
        let _ = fs::write(path, report.to_string());
    }
    #[cfg(windows)]
    if !quiet {
        use windows::{Win32::UI::WindowsAndMessaging::*, core::HSTRING};
        let message = result
            .as_ref()
            .map_or_else(|e| format!("Pelagic: {e}"), Clone::clone);
        if !message.is_empty() {
            unsafe {
                MessageBoxW(
                    None,
                    &HSTRING::from(message),
                    windows::core::w!("Pelagic"),
                    if result.is_ok() {
                        MB_OK
                    } else {
                        MB_OK | MB_ICONERROR
                    },
                );
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = quiet;
        println!("{result:?}");
    }
    if result.is_err() {
        std::process::exit(1);
    }
}

//! Windows builds embed the packaged ConPTY that `scripts/fetch-conpty.ps1`
//! fetches into `.conpty/<version>/`, and copy it next to the built exe so
//! `cargo run` loads it from there. They also compile the icon and version
//! resources. Other targets do nothing here.

use std::path::{Path, PathBuf};

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        conpty();
        // embed-resource (a Windows-only build dependency) is only there
        // when building on Windows, the only host Brindle builds Windows on.
        #[cfg(windows)]
        resources();
    }
}

/// Compiles `assets/windows/brindle.rc` into the exe, with the crate's
/// version and, once rendered, the icon.
#[cfg(windows)]
fn resources() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let dir = manifest.join("assets").join("windows");
    let (rc, ico) = (dir.join("brindle.rc"), dir.join("brindle.ico"));
    println!("cargo:rerun-if-changed={}", rc.display());
    println!("cargo:rerun-if-changed={}", ico.display());
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let number = |name: &str| std::env::var(name).unwrap();
    let mut macros = vec![
        format!("BRINDLE_VERSION=\"{version}\""),
        format!("BRINDLE_MAJOR={}", number("CARGO_PKG_VERSION_MAJOR")),
        format!("BRINDLE_MINOR={}", number("CARGO_PKG_VERSION_MINOR")),
        format!("BRINDLE_PATCH={}", number("CARGO_PKG_VERSION_PATCH")),
    ];
    if ico.is_file() {
        // rc.exe reads the path as a C string, so its backslashes are doubled.
        macros.push(format!("BRINDLE_ICON=\"{}\"", ico.display().to_string().replace('\\', "\\\\")));
    } else {
        println!("cargo:warning=no {}: brindle.exe gets no icon (run scripts/render-icons.sh)", ico.display());
    }
    embed_resource::compile(&rc, &macros).manifest_optional().expect("compiling assets/windows/brindle.rc");
}

/// Writes `$OUT_DIR/conpty.rs`: the pinned version and, when it has been
/// fetched, `conpty.dll` and `OpenConsole.exe` as byte arrays.
fn conpty() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let script = manifest.join("scripts").join("fetch-conpty.ps1");
    let fetched = manifest.join(".conpty");
    println!("cargo:rerun-if-changed={}", script.display());
    // Committed (it holds a README), so watching it never forces a rerun.
    println!("cargo:rerun-if-changed={}", fetched.display());

    let text = std::fs::read_to_string(&script).expect("scripts/fetch-conpty.ps1");
    let version = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("$Version = '")?.strip_suffix('\''))
        .expect("scripts/fetch-conpty.ps1 sets $Version");
    let dir = fetched.join(version);
    let (dll, host) = (dir.join("conpty.dll"), dir.join("OpenConsole.exe"));

    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let files = if dll.is_file() && host.is_file() {
        copy_beside_exe(&out, &[&dll, &host]);
        format!("Some((include_bytes!({dll:?}), include_bytes!({host:?})))")
    } else {
        println!(
            "cargo:warning=no packaged ConPTY in {}: run scripts/fetch-conpty.ps1, or local tabs use the ConPTY built into Windows",
            dir.display()
        );
        "None".to_string()
    };
    let code = format!(
        "/// The packaged ConPTY's version, from `scripts/fetch-conpty.ps1`.\n\
         pub const VERSION: &str = {version:?};\n\
         /// `conpty.dll` and `OpenConsole.exe`, when they were fetched.\n\
         pub const FILES: Option<(&[u8], &[u8])> = {files};\n"
    );
    std::fs::write(out.join("conpty.rs"), code).expect("writing conpty.rs");
}

/// Copies `files` into the profile directory (`target/<profile>/`), where
/// the exe is built. `OUT_DIR` is `target/<profile>/build/<pkg>-<hash>/out`.
fn copy_beside_exe(out: &Path, files: &[&Path]) {
    let Some(profile_dir) = out.ancestors().nth(3) else { return };
    for file in files {
        let target = profile_dir.join(file.file_name().unwrap());
        let same = std::fs::read(&target).is_ok_and(|bytes| std::fs::read(file).is_ok_and(|ours| ours == bytes));
        if !same {
            // A running Brindle holds the DLL open; the next build retries.
            if let Err(err) = std::fs::copy(file, &target) {
                println!("cargo:warning=copying {} to {}: {err}", file.display(), target.display());
            }
        }
    }
}

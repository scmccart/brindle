//! Windows builds compile the icon and version resources into the exe.
//! Other targets do nothing here. (The packaged ConPTY is embedded in the
//! `brindle-remote` helper, by `remote-agent/build.rs`.)

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
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
    use std::path::PathBuf;

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

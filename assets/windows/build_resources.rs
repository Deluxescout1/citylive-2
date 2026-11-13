// Shared by the build scripts of the two Windows programs (`include!`d).
// Embeds the icon, the manifest (per-monitor v2 DPI awareness, UTF-8, long
// paths) and the version information with mingw's windres. A no-op for every
// other target.

fn embed_windows_resources(description: &str, original_name: &str) {
    use std::path::PathBuf;
    use std::process::Command;
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../assets/windows")
        .canonicalize()
        .expect("assets/windows");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let fwd = |p: PathBuf| p.display().to_string().replace('\\', "/");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".into());
    let mut nums: Vec<u32> = version
        .split('.')
        .filter_map(|p| p.split('-').next().and_then(|n| n.parse().ok()))
        .collect();
    nums.resize(4, 0);
    let v = format!("{},{},{},{}", nums[0], nums[1], nums[2], nums[3]);
    let rc = format!(
        r#"1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {v}
PRODUCTVERSION {v}
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Deluxescout1"
      VALUE "FileDescription", "{description}"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "{original_name}"
      VALUE "OriginalFilename", "{original_name}"
      VALUE "LegalCopyright", "Copyright (c) Deluxescout1. MIT licence."
      VALUE "ProductName", "Citylive 2"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 0x04B0
  END
END
"#,
        icon = fwd(root.join("citylive.ico")),
        manifest = fwd(root.join("citylive.manifest")),
    );
    let rc_path = out.join("citylive.rc");
    std::fs::write(&rc_path, rc).unwrap();
    let obj = out.join("citylive_res.o");
    let windres = std::env::var("CITYLIVE_WINDRES")
        .unwrap_or_else(|_| "x86_64-w64-mingw32-windres".to_string());
    let status = Command::new(&windres)
        .arg("--input")
        .arg(&rc_path)
        .arg("--output")
        .arg(&obj)
        .arg("--output-format=coff")
        .status()
        .unwrap_or_else(|e| panic!("cannot run {windres}: {e}"));
    assert!(status.success(), "windres failed");
    println!("cargo:rustc-link-arg-bins={}", obj.display());
    println!("cargo:rerun-if-changed={}", root.join("citylive.ico").display());
    println!("cargo:rerun-if-changed={}", root.join("citylive.manifest").display());
    println!("cargo:rerun-if-changed={}", root.join("build_resources.rs").display());
}

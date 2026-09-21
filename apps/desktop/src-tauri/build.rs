use std::path::PathBuf;

fn main() {
    inject_product_version();

    let mut attributes = tauri_build::Attributes::new();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // tauri-build's default resource targets only the app binary. Linking the
        // manifest here also covers test executables that load Common Controls v6.
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        embed_windows_manifest();
    }

    tauri_build::try_build(attributes).expect("failed to run Tauri build script");
}

/// 产品版本的唯一来源是仓库根 package.json（见 docs/release-process.md 第 4 节），
/// Rust 侧对外标识（如 AI 协议 clientInfo）必须使用该版本而非内部 crate 版本。
/// 构建脚本在此把它注入为编译期常量 NOCTERM_PRODUCT_VERSION，供源码 env! 读取。
fn inject_product_version() {
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory is missing"),
    );
    // 与 tauri.conf.json 的 version 引用保持同一相对路径，维持单一版本源配置。
    let package_json = manifest_dir.join("../../../package.json");
    println!("cargo:rerun-if-changed={}", package_json.display());

    let content = std::fs::read_to_string(&package_json)
        .unwrap_or_else(|error| panic!("无法读取产品版本源 {}: {error}", package_json.display()));
    let version = serde_json::from_str::<serde_json::Value>(&content)
        .ok()
        .and_then(|value| value.get("version")?.as_str().map(str::to_string))
        .unwrap_or_else(|| panic!("产品版本源 {} 缺少字符串 version", package_json.display()));
    println!("cargo:rustc-env=NOCTERM_PRODUCT_VERSION={version}");
}

fn embed_windows_manifest() {
    let manifest = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory is missing"),
    )
    .join("windows-app-manifest.xml");

    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    // A missing or malformed manifest must fail the build instead of producing
    // a Windows executable that crashes before its test harness can start.
    println!("cargo:rustc-link-arg=/WX");
}

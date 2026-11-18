//! 把 `assets/airpane.rc` 里的应用图标编进可执行文件。
//!
//! 不引入额外的构建依赖：`rc.exe` 随 Windows SDK 安装，这里直接找最高版本的 x64 版本，
//! 编出 `.res` 后交给链接器。找不到就给出明确报错，而不是悄悄生成一个没有图标的 exe。

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let rc_file = assets.join("airpane.rc");
    println!("cargo:rerun-if-changed={}", rc_file.display());
    println!(
        "cargo:rerun-if-changed={}",
        assets.join("airpane.ico").display()
    );

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("缺少 OUT_DIR"));
    let res_file = out_dir.join("airpane.res");

    let rc = find_rc().unwrap_or_else(|| {
        panic!("找不到 Windows SDK 的 rc.exe（%ProgramFiles(x86)%\\Windows Kits\\10\\bin\\*\\x64\\rc.exe）")
    });
    let status = Command::new(&rc)
        .arg("/nologo")
        .arg(format!("/fo{}", res_file.display()))
        // `.rc` 里的 `airpane.ico` 是相对路径，按资源目录解析。
        .current_dir(&assets)
        .arg(&rc_file)
        .status()
        .unwrap_or_else(|err| panic!("运行 {} 失败：{err}", rc.display()));

    assert!(status.success(), "rc.exe 编译图标资源失败：{status}");
    println!("cargo:rustc-link-arg={}", res_file.display());
}

/// 在 `%ProgramFiles(x86)%\Windows Kits\10\bin\<版本>\x64\rc.exe` 里找版本最高的一个。
fn find_rc() -> Option<PathBuf> {
    let kits = std::env::var_os("ProgramFiles(x86)")?;
    let bin = Path::new(&kits).join("Windows Kits").join("10").join("bin");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(bin)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("x64").join("rc.exe"))
        .filter(|path| path.is_file())
        .collect();
    // 版本号目录排序后取最后一个，即最新的 SDK。
    candidates.sort();
    candidates.pop()
}

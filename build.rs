use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/claudie.manifest");

    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let icon_path = manifest_dir.join("assets").join("icon.ico");
    let app_manifest_path = manifest_dir.join("assets").join("claudie.manifest");
    if !icon_path.exists() {
        println!(
            "cargo:warning=Windows icon was not embedded because {} does not exist",
            icon_path.display()
        );
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("out dir"));
    let rc_path = out_dir.join("claudie.rc");
    let res_path = out_dir.join("claudie.res");
    let icon = icon_path.display().to_string().replace('\\', "\\\\");
    let app_manifest = app_manifest_path
        .display()
        .to_string()
        .replace('\\', "\\\\");
    let resource_script = if app_manifest_path.exists() {
        format!("1 ICON \"{icon}\"\n1 24 \"{app_manifest}\"\n")
    } else {
        println!(
            "cargo:warning=Windows visual styles manifest was not embedded because {} does not exist",
            app_manifest_path.display()
        );
        format!("1 ICON \"{icon}\"\n")
    };
    fs::write(&rc_path, resource_script).expect("write resource script");

    let Some(rc) = find_resource_compiler() else {
        println!(
            "cargo:warning=Windows icon was not embedded because rc.exe/llvm-rc.exe was not found"
        );
        return;
    };

    let status = Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_path)
        .arg(&rc_path)
        .status()
        .expect("run resource compiler");

    if !status.success() {
        panic!("resource compiler failed with status {status}");
    }

    println!("cargo:rustc-link-arg-bins={}", res_path.display());
}

fn find_resource_compiler() -> Option<PathBuf> {
    env::var_os("RC")
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .or_else(|| find_on_path("rc.exe"))
        .or_else(|| find_on_path("llvm-rc.exe"))
        .or_else(find_windows_sdk_rc)
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH")?
        .to_string_lossy()
        .split(';')
        .find_map(|dir| {
            let path = Path::new(dir).join(name);
            path.exists().then_some(path)
        })
}

fn find_windows_sdk_rc() -> Option<PathBuf> {
    let root = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut candidates = Vec::new();
    for version in fs::read_dir(root).ok()? {
        let version = version.ok()?.path();
        for arch in ["x64", "x86", "arm64"] {
            let rc = version.join(arch).join("rc.exe");
            if rc.exists() {
                candidates.push(rc);
            }
        }
    }
    candidates.sort();
    candidates.pop()
}

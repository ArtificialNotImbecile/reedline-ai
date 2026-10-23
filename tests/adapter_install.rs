use std::{
    fs,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn temporary() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "reedline-install-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir(&path).unwrap();
    path
}
fn run(action: &str, path: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_reedline-ai"))
        .args(["adapter", action, "--shell", "bash", "--directory"])
        .arg(path)
        .output()
        .unwrap()
}
#[test]
fn bundled_assets_install_without_checkout_and_uninstall_exactly() {
    let root = temporary();
    let path = root.join("adapter with spaces");
    assert!(run("install", &path).status.success());
    assert!(
        fs::read_to_string(path.join("reedline-ai.bash"))
            .unwrap()
            .contains("__rai_invoke")
    );
    assert!(
        fs::read_to_string(path.join("bridge.py"))
            .unwrap()
            .contains("No shell=True")
    );
    assert!(!run("install", &path).status.success());
    assert!(run("uninstall", &path).status.success());
    assert!(!path.exists());
    fs::remove_dir(root).unwrap();
}
#[test]
fn uninstall_preserves_modified_or_extra_files() {
    let root = temporary();
    let path = root.join("adapter");
    assert!(run("install", &path).status.success());
    fs::write(path.join("extra"), b"private").unwrap();
    assert!(!run("uninstall", &path).status.success());
    assert!(path.join("reedline-ai.bash").exists());
    fs::remove_file(path.join("extra")).unwrap();
    fs::write(path.join("bridge.py"), b"custom edits").unwrap();
    assert!(!run("uninstall", &path).status.success());
    assert!(path.join("reedline-ai.bash").exists());
    fs::remove_dir_all(root).unwrap();
}
#[cfg(unix)]
#[test]
fn install_and_uninstall_refuse_symlinks() {
    use std::os::unix::fs::symlink;
    let root = temporary();
    let target = root.join("target");
    fs::create_dir(&target).unwrap();
    let link = root.join("link");
    symlink(&target, &link).unwrap();
    assert!(!run("install", &link).status.success());
    assert!(!run("uninstall", &link).status.success());
    assert!(target.exists());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn experimental_shell_requires_explicit_acknowledgment() {
    let root = temporary();
    let path = root.join("adapter");
    let result = Command::new(env!("CARGO_BIN_EXE_reedline-ai"))
        .args(["adapter", "install", "--shell", "cmd", "--directory"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!path.exists());
    fs::remove_dir(root).unwrap();
}

#[test]
fn experimental_packages_are_self_contained_and_removed_safely() {
    for (shell, entry, helper) in [
        ("powershell", "ReedlineAI.psm1", "AdapterProcess.cs"),
        ("nushell", "nushell/reedline-ai.nu", "shared/bridge.py"),
        ("cmd", "cmd/reedline-ai.lua", "shared/bridge.py"),
    ] {
        let root = temporary();
        let path = root.join("experimental package");
        let result = Command::new(env!("CARGO_BIN_EXE_reedline-ai"))
            .args([
                "adapter",
                "install",
                "--shell",
                shell,
                "--experimental",
                "--directory",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(path.join(entry).is_file());
        assert!(path.join(helper).is_file());
        let result = Command::new(env!("CARGO_BIN_EXE_reedline-ai"))
            .args(["adapter", "uninstall", "--shell", shell, "--directory"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(!path.exists());
        fs::remove_dir(root).unwrap();
    }
}
#[test]
fn nested_package_extra_file_prevents_any_removal() {
    let root = temporary();
    let path = root.join("nu");
    assert!(
        Command::new(env!("CARGO_BIN_EXE_reedline-ai"))
            .args([
                "adapter",
                "install",
                "--shell",
                "nushell",
                "--experimental",
                "--directory"
            ])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    fs::write(path.join("shared/personal.py"), b"retain").unwrap();
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_reedline-ai"))
            .args(["adapter", "uninstall", "--shell", "nushell", "--directory"])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    assert!(path.join("nushell/reedline-ai.nu").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn nested_package_directory_symlink_is_not_followed() {
    use std::os::unix::fs::symlink;
    let root = temporary();
    let path = root.join("nu");
    assert!(
        Command::new(env!("CARGO_BIN_EXE_reedline-ai"))
            .args([
                "adapter",
                "install",
                "--shell",
                "nushell",
                "--experimental",
                "--directory"
            ])
            .arg(&path)
            .output()
            .unwrap()
            .status
            .success()
    );
    let real = root.join("retained-shared");
    fs::rename(path.join("shared"), &real).unwrap();
    symlink(&real, path.join("shared")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_reedline-ai"))
        .args(["adapter", "uninstall", "--shell", "nushell", "--directory"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(real.join("bridge.py").exists());
    assert!(path.join("nushell/reedline-ai.nu").exists());
    fs::remove_dir_all(root).unwrap();
}

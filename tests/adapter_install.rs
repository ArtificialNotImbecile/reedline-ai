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
fn unsupported_shell_is_not_claimed_installed() {
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

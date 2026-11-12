use super::*;
use crate::bootstrap::proton_install;

#[test]
fn proton_install_needs_manifest_and_accepts_dir_or_exe() {
    let root = TempDir::new("proton");
    std::fs::write(root.join("proton"), b"").expect("write proton");
    assert!(proton_install(root.path()).is_none());

    std::fs::write(root.join("toolmanifest.vdf"), b"").expect("write manifest");
    let from_dir = proton_install(root.path()).expect("dir install");
    assert_eq!(from_dir.root, root.path());
    assert_eq!(
        from_dir.name,
        root.path().file_name().unwrap().to_string_lossy()
    );

    let from_exe = proton_install(&root.join("proton")).expect("exe install");
    assert_eq!(from_exe.root, root.path());

    // 上层目录不是 Proton 时，指向不存在的可执行文件也无效
    let bare = TempDir::new("proton-bare");
    std::fs::create_dir_all(bare.join("nested")).expect("create nested dir");
    assert!(proton_install(&bare.join("nested/proton")).is_none());
}

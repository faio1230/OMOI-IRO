//! `omoiiro-probe check-shows` over the repository's show files (synthetic file check).

use std::process::Command;

fn check(paths: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_omoiiro-probe")).arg("check-shows").args(paths).output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn every_fixture_and_example_passes() {
    let (ok, text) = check(&[concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/shows"), concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples")]);
    assert!(ok, "{text}");
    assert!(text.contains(", 0 failed"), "{text}");
}

#[test]
fn a_broken_show_fails_with_its_reason() {
    let dir = std::env::temp_dir().join(format!("omoiiro-check-shows-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/demo.show.json")).unwrap();
    std::fs::write(dir.join("good.show.json"), &good).unwrap();
    std::fs::write(dir.join("broken.show.json"), good.replacen("\"preset_id\": \"base-blue\"", "\"preset_id\": \"missing\"", 1)).unwrap();
    std::fs::write(dir.join("notes.json"), "{}").unwrap();
    let (ok, text) = check(&[dir.to_str().unwrap()]);
    std::fs::remove_dir_all(&dir).ok();
    assert!(!ok);
    assert!(text.contains("[FAIL]") && text.contains("broken.show.json") && text.contains("unknown preset"), "{text}");
    assert!(text.contains("checked 2 files, 1 failed"), "only *.show.json: {text}");
}
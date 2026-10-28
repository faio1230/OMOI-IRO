//! `omoiiro-probe render` on examples/demo.show.json (headless, task C10).

use std::process::Command;

fn render(args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_omoiiro-probe"))
        .args(["render", "--show", concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/demo.show.json"), "--json"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn demo_renders_at_any_time_with_label_and_grid() {
    let v = render(&["--at", "9000000", "--beat", "0.25"]);
    assert_eq!(v["show_id"], "demo-show");
    assert_eq!(v["mtc_label"], "01:00:09:00");
    assert_eq!(v["rows_top_down"].as_array().unwrap().len(), 8);
    assert_eq!(v["leds_bottom_left"].as_array().unwrap().len(), 64);
    // The sweep band (2 columns) differs from the base color.
    let row: Vec<&str> = v["rows_top_down"][0].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
    let distinct: std::collections::HashSet<&str> = row.iter().copied().collect();
    assert_eq!(distinct.len(), 2, "{row:?}");
}

#[test]
fn before_the_song_start_is_black_and_deterministic() {
    let a = render(&["--at", "-1"]);
    assert!(a["leds_bottom_left"].as_array().unwrap().iter().all(|c| c == "#000000"));
    assert_eq!(render(&["--at", "31999999", "--beat", "7.3"]), render(&["--at", "31999999", "--beat", "7.3"]));
}

#[test]
fn invalid_show_is_reported() {
    let dir = std::env::temp_dir().join(format!("render-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bad.show.json");
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/demo.show.json"))
        .unwrap()
        .replace("\"opacity\": 0.8", "\"opacity\": 1.8");
    std::fs::write(&path, text).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_omoiiro-probe")).args(["render", "--show", path.to_str().unwrap(), "--at", "0"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("/layers/1/opacity"));
    std::fs::remove_dir_all(&dir).ok();
}

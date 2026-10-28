use std::process::Command;

#[test]
fn two_client_processes_compose_a_menu_beyond_its_window_offscreen() {
    let out = std::env::temp_dir().join(format!("oxyos-offscreen-test-{}", std::process::id()));
    let output = Command::new(env!("CARGO_BIN_EXE_oxyos-offscreen"))
        .arg("--out")
        .arg(&out)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    for fact in [
        "\"mode\":\"offscreen-cpu\"",
        "\"drm\":false",
        "\"bloom\":false",
        "\"popup_beyond_parent_edges\":4",
        "\"menu_row_pressed_over_other_client\":true",
        "\"outside_presses_delivered\":0",
        "\"descriptors_settled\":true",
        "\"status\":\"passed\"",
    ] {
        assert!(stdout.contains(fact), "missing {fact} in {stdout}");
    }
    for snapshot in [
        "1-windows.png",
        "2-menu-open.png",
        "3-menu-row-pressed.png",
        "4-menu-dismissed.png",
    ] {
        let bytes = std::fs::read(out.join(snapshot)).unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
    }
    std::fs::remove_dir_all(&out).unwrap();
}

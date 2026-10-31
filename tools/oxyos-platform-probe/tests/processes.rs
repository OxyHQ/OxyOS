use std::process::Command;

#[test]
fn two_independent_processes_negotiate_present_and_open_a_popup_without_a_display_server() {
    let output = Command::new(env!("CARGO_BIN_EXE_oxyos-platform-probe"))
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
    assert_eq!(stdout.trim(), "{\"mode\":\"headless-metadata\",\"protocol\":7,\"clients\":2,\"frames\":7,\"popups_beyond_parent\":1,\"graphics\":false,\"bloom\":false,\"status\":\"passed\"}");
}

fn main() {
    // Declaring the app's commands makes Tauri require an explicit
    // permission for each one (capabilities/main.json): the webview can call
    // these and nothing else (no shell, no fs, no plugin commands).
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "status",
            "wordlist",
            "add_folder",
            "remove_folder",
            "create_invite",
            "cancel_invite",
            "dismiss_invite",
            "revoke_device",
            "check_link",
            "pair_with",
            "list_dir",
            "get_file",
            "resume_transfer",
            "clear_finished",
            "open_file",
            "show_in_folder",
            "forget_computer",
            "rename_computer",
        ]),
    ))
    .expect("tauri build step failed");
}

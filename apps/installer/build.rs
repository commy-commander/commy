const BUILD_VALUES: [&str; 2] = ["COMMY_INSTALL_CHANNEL", "COMMY_RELEASES_URL"];

fn main() {
    for name in BUILD_VALUES {
        println!("cargo:rerun-if-env-changed={name}");
    }
    tauri_build::build()
}

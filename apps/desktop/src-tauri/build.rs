const BUILD_VALUES: [&str; 5] = ["COMMY_TEST_BUILD", "COMMY_DEFAULT_ACCOUNT_SERVER", "COMMY_VERSION", "COMMY_RELEASES_URL", "COMMY_LINK_SCHEME"];
const FORWARDED_PREFIX: &str = "TAURI_";

fn forward_build_values() {
    for name in BUILD_VALUES {
        let forwarded = format!("{FORWARDED_PREFIX}{name}");
        println!("cargo:rerun-if-env-changed={name}");
        println!("cargo:rerun-if-env-changed={forwarded}");
        if std::env::var_os(name).is_none() {
            if let Ok(value) = std::env::var(&forwarded) {
                println!("cargo:rustc-env={name}={value}");
            }
        }
    }
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-arg=-lc++abi");
    }
    forward_build_values();
    tauri_build::build()
}

use cfg_aliases::cfg_aliases;

const BUILD_VALUES: [&str; 2] = ["COMMY_DEFAULT_RELAY", "COMMY_RELAY_QAD_PORT"];
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
    cfg_aliases! {
        wasm_browser: { all(target_family = "wasm", target_os = "unknown") },
    }
    forward_build_values();
}

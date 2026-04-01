use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("protocol_version.rs");

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32;

    fs::write(
        &dest_path,
        format!("pub const PROTOCOL_VERSION: u32 = {timestamp};\n"),
    )
    .unwrap();

    println!("cargo:rerun-if-changed=build.rs");
}

use std::{env, fs, path::PathBuf};

fn main() {
    if env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }

    println!("cargo:rerun-if-changed=assets/app-icon.ico");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo did not set OUT_DIR"));
    let resource = out_dir.join("app-icon.rc");
    fs::copy("assets/app-icon.ico", out_dir.join("app-icon.ico"))
        .expect("Failed to stage the Windows application icon");
    fs::write(&resource, "1 ICON \"app-icon.ico\"\n")
        .expect("Failed to write the Windows resource script");
    embed_resource::compile(&resource, embed_resource::NONE)
        .manifest_required()
        .expect("Failed to embed the Windows application icon");
}

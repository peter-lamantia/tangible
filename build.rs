fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        println!("cargo:rerun-if-changed=app-icon.rc");
        println!("cargo:rerun-if-changed=assets/app-icon.ico");
        embed_resource::compile("app-icon.rc", embed_resource::NONE)
            .manifest_required()
            .expect("Failed to embed the Windows application icon");
    }
}

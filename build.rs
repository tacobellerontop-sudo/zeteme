fn main() {
    println!("cargo:rerun-if-changed=assets/zeteme.rc");
    println!("cargo:rerun-if-changed=assets/zeteme.ico");
    // Only Windows builds carry the icon. A Windows host must manage it; from
    // elsewhere (clippy for the Windows target) it is fine to go without.
    let result = embed_resource::compile("assets/zeteme.rc", embed_resource::NONE);
    let result = if cfg!(windows) {
        result.manifest_required()
    } else {
        result.manifest_optional()
    };
    if let Err(err) = result {
        panic!("could not embed zeteme's icon: {err}");
    }
}

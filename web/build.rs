/// Tell cargo to recompile when static files change.
/// This ensures include_str!/include_bytes! pick up the latest versions.
fn main() {
    println!("cargo::rerun-if-changed=static/");
}

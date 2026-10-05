fn main() {
    println!("cargo:rerun-if-changed=../suite.json");
    tauri_build::build()
}

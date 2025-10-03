fn main() {
    println!("cargo:rerun-if-changed=schema/migrations");
}

fn main() {
    println!("cargo:rerun-if-changed=tests/c_harness.c");
    println!("cargo:rerun-if-changed=include/narrata.h");
    cc::Build::new()
        .file("tests/c_harness.c")
        .include("include")
        .warnings(true)
        .compile("narrata_c_harness");
}

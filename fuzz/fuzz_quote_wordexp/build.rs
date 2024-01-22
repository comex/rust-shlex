fn main() {
    println!("cargo:rerun-if-changed=src/wordexp_wrapper.c");
    cc::Build::new()
        .file("src/wordexp_wrapper.c")
        .compile("wordexp_wrapper");
}


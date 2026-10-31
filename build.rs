fn main() {
    #[cfg(windows)]
    {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let assets_dir = std::path::Path::new(&manifest_dir).join("assets");

        // Link the embedded Windows icon & version resource directly into the binary
        let res_path = assets_dir.join("dropit_res.o");
        if res_path.exists() {
            let p = res_path.display();
            println!("cargo:rustc-link-arg-bin=dropit={}", p);
            println!("cargo:rustc-link-arg={}", p);
        }

        // On gnullvm/MinGW, statically link libunwind by prioritizing local assets/libunwind.a
        // over the toolchain's dynamic libunwind.dll.a
        if assets_dir.join("libunwind.a").exists() {
            println!("cargo:rustc-link-search=native={}", assets_dir.display());
        }

        println!("cargo:rerun-if-changed=build.rs");
        println!("cargo:rerun-if-changed=assets/dropit_res.o");
        println!("cargo:rerun-if-changed=assets/libunwind.a");
    }
}

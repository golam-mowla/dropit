fn main() {
    #[cfg(windows)]
    {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let assets_dir = std::path::Path::new(&manifest_dir).join("assets");

        // Link the embedded Windows icon & version resource directly into the application binary
        let res_path = assets_dir.join("dropit_res.o");
        if res_path.exists() {
            let p = res_path.display();
            println!("cargo:rustc-link-arg-bin=dropit={}", p);
        }

        // On gnullvm/MinGW, statically link libunwind by directly passing assets/libunwind.a
        let libunwind_path = assets_dir.join("libunwind.a");
        if libunwind_path.exists() {
            println!("cargo:rustc-link-search=native={}", assets_dir.display());
            println!(
                "cargo:rustc-link-arg-bin=dropit={}",
                libunwind_path.display()
            );
            println!("cargo:rustc-link-arg-bin=dropit=-Wl,-Bstatic");
            println!("cargo:rustc-link-arg-bin=dropit=-lunwind");
            println!("cargo:rustc-link-arg-bin=dropit=-Wl,-Bdynamic");
        }

        println!("cargo:rerun-if-changed=build.rs");
        println!("cargo:rerun-if-changed=assets/dropit_res.o");
        println!("cargo:rerun-if-changed=assets/libunwind.a");
    }
}

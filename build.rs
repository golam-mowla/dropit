fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/dropit.ico");
        res.set("ProductName", "Drop It");
        res.set(
            "FileDescription",
            "Drop It - Fast, Lightweight Local WiFi File Transfer for Windows",
        );
        res.set("LegalCopyright", "Copyright (c) 2026 Golam Mowla");
        res.set("CompanyName", "Golam Mowla");
        res.set("OriginalFilename", "dropit.exe");
        if let Err(e) = res.compile() {
            eprintln!("cargo:warning=Failed to compile Windows resources: {}", e);
        }
    }
}

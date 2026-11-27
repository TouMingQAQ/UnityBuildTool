fn main() {
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("icons/icon.ico");
        resource.set("ProductName", "UnityBuildTool");
        resource.set("CompanyName", "unitybuildtool");
        resource.set("FileDescription", "UnityBuildTool 管理端");
        resource.set("FileVersion", env!("CARGO_PKG_VERSION"));
        resource.set("ProductVersion", env!("CARGO_PKG_VERSION"));
        resource.set("OriginalFilename", "unitybuildtool-admin.exe");
        resource.set("LegalCopyright", "Copyright (c) 2026 unitybuildtool");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}

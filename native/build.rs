fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-dark".into()).with_bundled_translations("lang");
    slint_build::compile_with_config("ui/app.slint", config).expect("failed to compile the Slint UI");

    // The icon of the .exe itself (Explorer, shortcuts, the taskbar when pinned).
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the exe icon: {e}");
        }
    }
}

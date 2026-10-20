fn main() {
    // Icon for brightness.exe in Explorer and the taskbar. The window icon on
    // all platforms is set at runtime in main.rs.
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("assets/icon.ico")
        .compile()
        .expect("embedding the Windows icon");
}

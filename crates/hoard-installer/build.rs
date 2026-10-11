fn main() {
    // `DefaultTranslationContext::None` so a string is keyed by the string
    // itself and not by the component it happens to sit in. Without it, moving
    // "Close" from one screen to another silently orphans its translation.
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations("lang")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/installer.slint", config)
        .expect("compiling the installer UI");

    // The file icon, and the details Windows shows under Properties. Both are
    // PE resources, so they are stamped at link time and not settable from the
    // UI at all.
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        res.set("ProductName", "Hoard");
        res.set("FileDescription", "Hoard Setup");
        res.set("CompanyName", "Hoard");
        res.set("LegalCopyright", "AGPL-3.0");
        // Windows takes anything whose description says "Setup" for an
        // installer and demands elevation before it runs, unless the program
        // states otherwise. Without this, double-clicking it asked for an
        // administrator to install into the user's own folder, and an older
        // Hoard's updater, which starts it with no shell to raise the prompt,
        // got "the requested operation requires elevation" and never updated.
        res.set_manifest(
            r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
</assembly>"#,
        );
        res.compile()
            .expect("embedding the Windows icon and version resource");
    }
}

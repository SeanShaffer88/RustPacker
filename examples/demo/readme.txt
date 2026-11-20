RustPacker demo project
=======================

Packs a two-file payload into a real single-file Windows installer.

  rustpacker-cli validate rustpacker.toml
  rustpacker-cli pack rustpacker.toml --stub path/to/rustpacker-stub.exe --portable

IMPORTANT: payload/DemoApp.exe is a text placeholder, not a program.

It exists so `app.main_exe` resolves inside the payload (packing fails
otherwise). The demo therefore exercises the *installer* end to end -- wizard,
progress, files, shortcut, Apps & Features entry, journal, uninstall -- while
the app it launches does nothing, because there is nothing to launch.

Replace payload/DemoApp.exe with a real executable of the same name for a
working install.

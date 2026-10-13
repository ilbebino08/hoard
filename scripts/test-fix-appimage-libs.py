#!/usr/bin/env python3
"""Exercise the AppImage release postprocessor with real SquashFS artifacts.

Run with HOARD_TEST_APPIMAGETOOL pointing to an official appimagetool AppImage.
The only substitute is its network download: reuse that local tool/runtime.
"""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
TOOL = os.environ.get("HOARD_TEST_APPIMAGETOOL")


@unittest.skipUnless(TOOL and shutil.which("mksquashfs"), "requires appimagetool and mksquashfs")
class AppImageReleaseTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.work = Path(self.scratch.name)
        self.env = {**os.environ, "APPIMAGELAUNCHER_DISABLE": "1", "ARCH": os.uname().machine}
        tool = Path(TOOL).resolve()
        offset = int(self.run_command([tool, "--appimage-offset"]).stdout.strip().splitlines()[-1])
        runtime = self.work / "runtime"
        runtime.write_bytes(tool.read_bytes()[:offset])

        # Keep the production repackaging tool real while removing network access.
        launcher = self.work / "tool-launcher"
        launcher.write_text(
            '#!/bin/sh\nexec "$HOARD_TEST_APPIMAGETOOL" "$@" '
            '--runtime-file "$HOARD_TEST_RUNTIME"\n'
        )
        launcher.chmod(0o755)
        bin_dir = self.work / "bin"
        bin_dir.mkdir()
        downloader = bin_dir / "wget"
        downloader.write_text(
            '#!/bin/sh\nwhile [ "$#" -gt 0 ]; do\n'
            '  if [ "$1" = "-O" ]; then cp "$HOARD_TEST_TOOL_LAUNCHER" "$2"; exit; fi\n'
            '  shift\ndone\nexit 1\n'
        )
        downloader.chmod(0o755)
        self.env.update(
            PATH=str(bin_dir) + os.pathsep + self.env["PATH"],
            HOARD_TEST_APPIMAGETOOL=str(tool),
            HOARD_TEST_RUNTIME=str(runtime),
            HOARD_TEST_TOOL_LAUNCHER=str(launcher),
        )

    def run_command(self, args, **kwargs):
        return subprocess.run(
            [str(arg) for arg in args], env=self.env, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, **kwargs,
        )

    def fixture(self, with_graphics=False, integration=None):
        appdir = self.work / "AppDir"
        desktop_dir = appdir / "usr/share/applications"
        desktop_dir.mkdir(parents=True)
        (appdir / "usr/lib").mkdir(parents=True)
        (appdir / "usr/lib/libayatana-appindicator3.so.1").write_bytes(b"tray fixture")
        if with_graphics:
            (appdir / "usr/lib/libwayland-client.so.0").write_bytes(b"host library fixture")
        desktop = desktop_dir / "hoard.desktop"
        integration_line = f"X-AppImage-Integrate={integration}\n" if integration else ""
        desktop.write_text(
            "[Desktop Entry]\nType=Application\nName=Hoard packaging fixture\n"
            "Exec=AppRun\nIcon=hoard\nTerminal=false\nCategories=Utility;\n"
            f"{integration_line}Actions=Test;\n\n"
            "[Desktop Action Test]\nName=Test\nExec=AppRun --test\n"
        )
        (appdir / "hoard.desktop").symlink_to("usr/share/applications/hoard.desktop")
        shutil.copyfile(ROOT / "crates/hoard-desktop/icons/128x128@2x.png", appdir / "hoard.png")
        (appdir / "AppRun").write_text("#!/bin/sh\nprintf 'hoard-packaging-fixture\\n'\n")
        (appdir / "AppRun").chmod(0o755)
        squashfs = self.work / "payload.squashfs"
        self.run_command([
            "mksquashfs", appdir, squashfs, "-noappend", "-all-root", "-processors", "2",
        ])
        bundle = self.work / "bundle/appimage"
        bundle.mkdir(parents=True)
        image = bundle / "Hoard.AppImage"
        image.write_bytes((self.work / "runtime").read_bytes() + squashfs.read_bytes())
        image.chmod(0o755)
        return image

    def assert_processed(self, with_graphics, integration=None):
        image = self.fixture(with_graphics, integration)
        self.run_command(["bash", ROOT / "scripts/fix-appimage-libs.sh", image.parent.parent])
        extract = self.work / "extracted"
        extract.mkdir()
        self.run_command([image, "--appimage-extract"], cwd=extract)
        appdir = extract / "squashfs-root"
        text = (appdir / "hoard.desktop").read_text()
        self.assertIn("X-AppImage-Integrate=false\n", text)
        self.assertNotIn("X-AppImage-Integrate=true", text)
        self.assertEqual(text.count("X-AppImage-Integrate="), 1)
        self.assertTrue((appdir / "hoard.desktop").is_symlink())
        self.assertIn("[Desktop Action Test]\nName=Test\nExec=AppRun --test", text)
        self.assertTrue((appdir / "usr/lib/libayatana-appindicator3.so.1").exists())
        self.assertFalse((appdir / "usr/lib/libwayland-client.so.0").exists())

    def test_opt_out_is_repackaged_when_no_graphics_libraries_need_removal(self):
        self.assert_processed(False)

    def test_opt_out_and_graphics_cleanup_are_both_applied(self):
        self.assert_processed(True, "true")

    def test_opt_out_is_not_duplicated_when_already_present(self):
        self.assert_processed(False, "false")


if __name__ == "__main__":
    unittest.main()

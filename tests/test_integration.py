"""Real mise -> Lua -> nested mise/uv -> Python -> mock Blob storage, no Azure access."""

import hashlib
import json
import os
import platform
import shutil
import stat
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path


class MiseIntegrationTests(unittest.TestCase):
	@unittest.skipUnless(shutil.which("mise") and shutil.which("uv"), "Requires mise and uv")
	def test_list_install_execute_and_reuse_login(self):
		repo = Path(__file__).resolve().parents[1]
		with tempfile.TemporaryDirectory(prefix="azure mise integration spaces ") as tmp:
			root = Path(tmp)
			project = root / "consumer project"
			project.mkdir()
			storage = root / "storage/hello/1.0.0"
			storage.mkdir(parents=True)
			host_os = {"Linux": "linux", "Darwin": "darwin", "Windows": "windows"}[
				platform.system()
			]
			arch = {"x86_64": "amd64", "AMD64": "amd64", "aarch64": "arm64", "arm64": "arm64"}[
				platform.machine()
			]
			binary_name = "hello.cmd" if os.name == "nt" else "hello"
			binary = (
				"@echo off\necho from-mock\n" if os.name == "nt" else "#!/bin/sh\necho from-mock\n"
			)
			archive = storage / "hello.zip"
			with zipfile.ZipFile(archive, "w") as output:
				item = zipfile.ZipInfo("bin/" + binary_name)
				item.external_attr = (stat.S_IFREG | 0o755) << 16
				output.writestr(item, binary)
			(storage / "release.json").write_text(
				json.dumps(
					{
						"platforms": {
							f"{host_os}-{arch}": {
								"file": archive.name,
								"sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
							}
						}
					}
				)
			)
			(storage.parent / "index.json").write_text(json.dumps({"versions": ["1.0.0"]}))
			(project / "mise.toml").write_text(
				"[plugins]\nazure-ac = " + json.dumps(str(repo)) + "\n[tools]\n"
				'"azure-ac:hello" = { version = "1.0.0", '
				'storage_url = "https://account.blob.core.windows.net/tools", auth = "browser" }\n'
			)
			env = {k: v for k, v in os.environ.items() if not k.startswith(("MISE_", "AZURE_"))}
			env.pop("CI", None)
			env.update(
				{
					"MISE_DATA_DIR": str(root / "mise-data"),
					"MISE_CACHE_DIR": str(root / "mise-cache"),
					"MISE_STATE_DIR": str(root / "mise-state"),
					"MISE_CONFIG_DIR": str(root / "mise-config"),
					"MISE_TRUSTED_CONFIG_PATHS": str(project),
					"MISE_YES": "1",
					"MISE_LOG_LEVEL": "warn",
					"XDG_DATA_HOME": str(root / "identity-data"),
					"PYTHONPATH": os.pathsep.join(
						[str(repo / "tests/mock_runtime"), str(repo / "tests")]
					),
					"MOCK_AZURE_STORAGE": str(storage.parent.parent),
					"MOCK_AZURE_LOGIN": str(root / "authenticated"),
				}
			)
			# Reuse an already installed uv in this isolated mise installation: no
			# downloads are needed by this test. The real consumer bootstraps it.
			uv = Path(shutil.which("uv")).resolve()
			uv_root = next(parent for parent in uv.parents if parent.name == "0.13.4")
			destination = root / "mise-data/installs/uv/0.13.4"
			destination.parent.mkdir(parents=True)
			if os.name == "nt":
				shutil.copytree(uv_root, destination)
			else:
				destination.symlink_to(uv_root, target_is_directory=True)

			def mise(*args, success=True):
				result = subprocess.run(
					[shutil.which("mise"), "-C", str(project), *args],
					env=env,
					text=True,
					capture_output=True,
					timeout=120,
				)
				if success:
					self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
				else:
					self.assertNotEqual(result.returncode, 0)
				return result

			# No manual plugin-link or plugin-install step: the project declares it.
			listing = mise("ls-remote", "azure-ac:hello")
			self.assertIn("1.0.0", listing.stdout)
			self.assertTrue((root / "authenticated").exists())
			mise("install")
			args = (
				("exec", "--", "cmd", "/c", "hello") if os.name == "nt" else ("exec", "--", "hello")
			)
			self.assertEqual(mise(*args).stdout.strip(), "from-mock")
			# The cached identity must also work in a noninteractive process.
			env["AZURE_AC_NONINTERACTIVE"] = "1"
			mise("cache", "clear", "azure-ac:hello")
			self.assertIn("1.0.0", mise("ls-remote", "azure-ac:hello").stdout)
			(root / "authenticated").unlink()
			mise("cache", "clear", "azure-ac:hello")
			failure = mise("ls-remote", "azure-ac:hello", success=False)
			self.assertIn("sign-in required", failure.stderr)


if __name__ == "__main__":
	unittest.main()

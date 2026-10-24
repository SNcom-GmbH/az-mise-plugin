import hashlib
import io
import json
import os
import stat
import sys
import tarfile
import tempfile
import types
import unittest
import zipfile
from pathlib import Path
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from mock_storage import Container

import azure_ac as ac


class PluginTests(unittest.TestCase):
	def setUp(self):
		self.temporary = tempfile.TemporaryDirectory(prefix="azure mock spaces ")
		self.addCleanup(self.temporary.cleanup)
		self.root = Path(self.temporary.name)
		self.container = Container(self.root / "storage")

	def put_json(self, name, value):
		self.container.upload_blob(name, json.dumps(value))

	def request(self):
		return {
			"tool": "hello",
			"version": "1.0.0",
			"platform": "linux-amd64",
			"download_path": str(self.root / "download with spaces"),
			"install_path": str(self.root / "install with spaces"),
		}

	def archive(self, name="bin/hello", kind="file", zip_format=False):
		data = io.BytesIO()
		if zip_format:
			with zipfile.ZipFile(data, "w") as archive:
				item = zipfile.ZipInfo(name)
				item.external_attr = (
					(stat.S_IFLNK if kind == "link" else stat.S_IFREG) | 0o755
				) << 16
				archive.writestr(item, b"hello world")
		else:
			with tarfile.open(fileobj=data, mode="w:gz") as archive:
				item = tarfile.TarInfo(name)
				item.mode = 0o755
				if kind == "link":
					item.type = tarfile.SYMTYPE
					item.linkname = "../../outside"
					archive.addfile(item)
				else:
					item.size = len(b"hello world")
					archive.addfile(item, io.BytesIO(b"hello world"))
		return data.getvalue()

	def release(self, data, digest=None, filename="hello.tar.gz"):
		self.container.upload_blob("hello/1.0.0/" + filename, data)
		self.put_json(
			"hello/1.0.0/release.json",
			{
				"platforms": {
					"linux-amd64": {
						"file": filename,
						"sha256": digest or hashlib.sha256(data).hexdigest(),
					}
				}
			},
		)

	def test_storage_url(self):
		self.assertEqual(
			ac.storage_url("https://account.blob.core.windows.net/tools/"),
			"https://account.blob.core.windows.net/tools",
		)
		for value in (
			"http://account.blob.core.windows.net/tools",
			"https://evil.invalid/tools",
			"https://account.blob.core.windows.net.evil.invalid/tools",
			"https://user@account.blob.core.windows.net/tools",
			"https://account.blob.core.windows.net/tools?sig=secret",
			"https://account.blob.core.windows.net/tools/path",
			"https://account.blob.core.windows.net/x",
			"https://account.blob.core.windows.net/t--ools",
		):
			with self.subTest(value=value), self.assertRaises(ValueError):
				ac.storage_url(value)

	def test_versions_preserve_release_order(self):
		entries = ["1.9.0", {"version": "1.10.0", "created_at": "2026-10-01T00:00:00Z"}]
		self.put_json("hello/index.json", {"versions": entries})
		self.assertEqual(ac.versions(self.container, "hello"), {"versions": entries})

	def test_invalid_versions(self):
		for entries in ([], ["../escape"], ["1", "1"], ["a/b"]):
			with self.subTest(entries=entries), self.assertRaises(ValueError):
				self.put_json("hello/index.json", {"versions": entries})
				ac.versions(self.container, "hello")
			(self.container.root / "hello/index.json").unlink()

	def test_install_tar(self):
		self.release(self.archive())
		request = self.request()
		self.assertEqual(ac.install(self.container, request), {})
		binary = Path(request["install_path"]) / "bin/hello"
		self.assertEqual(binary.read_bytes(), b"hello world")
		self.assertTrue(binary.stat().st_mode & 0o111)

	def test_install_zip(self):
		self.release(self.archive(zip_format=True), filename="hello.zip")
		ac.install(self.container, self.request())
		self.assertTrue((self.root / "install with spaces/bin/hello").exists())

	def test_checksum_mismatch_installs_nothing(self):
		self.release(self.archive(), "0" * 64)
		with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
			ac.install(self.container, self.request())
		self.assertFalse(Path(self.request()["install_path"]).exists())

	def test_missing_platform(self):
		self.release(self.archive())
		request = self.request() | {"platform": "windows-arm64"}
		with self.assertRaisesRegex(ValueError, "no artifact"):
			ac.install(self.container, request)

	def test_archive_traversal_and_links(self):
		for zip_format in (False, True):
			for name, kind in (
				("../outside", "file"),
				("/absolute", "file"),
				("bin/link", "link"),
				("bin/NUL", "file"),
				("bin/file:ads", "file"),
				("bin/foo.", "file"),
			):
				with self.subTest(zip=zip_format, name=name):
					archive = self.root / ("bad.zip" if zip_format else "bad.tar.gz")
					archive.write_bytes(self.archive(name, kind, zip_format))
					with self.assertRaises(ValueError):
						ac.extract(archive, self.root / "extracted")
		self.assertFalse((self.root / "outside").exists())

	def test_no_bin_directory(self):
		self.release(self.archive("hello"))
		with self.assertRaisesRegex(ValueError, "bin/"):
			ac.install(self.container, self.request())

	def test_windows_uses_broker(self):
		broker = types.ModuleType("azure.identity.broker")
		broker.InteractiveBrowserBrokerCredential = MagicMock()
		with (
			patch.dict(sys.modules, {"azure.identity.broker": broker}),
			patch.object(sys, "platform", "win32"),
			patch.dict(os.environ, {"XDG_DATA_HOME": str(self.root)}, clear=True),
		):
			ac.credential({})
		kwargs = broker.InteractiveBrowserBrokerCredential.call_args.kwargs
		self.assertTrue(kwargs["use_default_broker_account"])
		self.assertTrue(kwargs["disable_automatic_authentication"])
		self.assertFalse(kwargs["cache_persistence_options"].allow_unencrypted_storage)

	def test_explicit_tenant_and_client(self):
		with (
			patch.object(ac, "InteractiveBrowserCredential") as factory,
			patch.dict(os.environ, {"XDG_DATA_HOME": str(self.root)}, clear=True),
		):
			ac.credential({"auth": "browser", "tenant_id": "tenant", "client_id": "client"})
		self.assertEqual(factory.call_args.kwargs["tenant_id"], "tenant")
		self.assertEqual(factory.call_args.kwargs["client_id"], "client")

	def test_ci_identity_does_not_fall_back_to_person(self):
		with (
			patch.object(ac, "EnvironmentCredential") as environment,
			patch.dict(os.environ, {"AZURE_CLIENT_SECRET": "fake"}, clear=True),
		):
			self.assertIs(ac.credential({}), environment.return_value)
		with (
			patch.object(ac, "WorkloadIdentityCredential") as workload,
			patch.dict(os.environ, {"AZURE_FEDERATED_TOKEN_FILE": "fake"}, clear=True),
		):
			self.assertIs(ac.credential({}), workload.return_value)

	def test_noninteractive_never_opens_prompt(self):
		fake = MagicMock(spec=ac.InteractiveBrowserCredential)
		fake.__enter__.return_value = fake
		fake.get_token.side_effect = ac.AuthenticationRequiredError(scopes=[ac.SCOPE])
		with (
			patch.object(ac, "credential", return_value=fake),
			patch.dict(os.environ, {"AZURE_AC_NONINTERACTIVE": "1"}),
			self.assertRaisesRegex(ValueError, "disabled"),
		):
			ac.authenticate({})
		fake.authenticate.assert_not_called()

	def test_list_upload_download_read(self):
		source = self.root / "source.txt"
		source.write_text("hello")
		cred = MagicMock()
		options = {"storage_url": "https://account.blob.core.windows.net/tools"}
		with (
			patch.object(ac, "credential", return_value=cred),
			patch.object(ac.ContainerClient, "from_container_url", return_value=self.container),
		):
			ac.dispatch(
				{"options": options, "action": "upload", "blob": "notes.txt", "file": str(source)}
			)
			self.assertEqual(
				ac.dispatch({"options": options, "action": "read", "blob": "notes.txt"}),
				{"text": "hello"},
			)
			result = ac.dispatch({"options": options, "action": "list"})
			self.assertEqual(result, [{"name": "notes.txt", "size": 5}])
			target = self.root / "target.txt"
			ac.dispatch(
				{"options": options, "action": "download", "blob": "notes.txt", "file": str(target)}
			)
			self.assertEqual(target.read_text(), "hello")
			with self.assertRaises(FileExistsError):
				ac.dispatch(
					{
						"options": options,
						"action": "upload",
						"blob": "notes.txt",
						"file": str(source),
					}
				)


if __name__ == "__main__":
	unittest.main()

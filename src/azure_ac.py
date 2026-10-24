"""Private Blob artifacts. Tokens stay inside Azure Identity, never in Lua or stdout."""

from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import re
import shutil
import stat
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path, PurePosixPath
from urllib.parse import unquote, urlsplit

from azure.core.exceptions import HttpResponseError
from azure.identity import (
	AuthenticationRecord,
	AuthenticationRequiredError,
	DeviceCodeCredential,
	EnvironmentCredential,
	InteractiveBrowserCredential,
	ManagedIdentityCredential,
	TokenCachePersistenceOptions,
	WorkloadIdentityCredential,
)
from azure.storage.blob import ContainerClient

SCOPE = "https://storage.azure.com/.default"
DEFAULT_CLIENT = "04b07795-8ddb-461a-bbee-02f9e1bf7b46"
logging.getLogger("azure").setLevel(logging.ERROR)
logging.getLogger("msal").setLevel(logging.ERROR)


def storage_url(value: str) -> str:
	"""Only public Azure cloud Blob endpoints; never send a token to arbitrary URLs."""
	parts = urlsplit(value)
	host = parts.hostname or ""
	if (
		parts.scheme != "https"
		or not re.fullmatch(r"[a-z0-9]{3,24}\.blob\.core\.windows\.net", host)
		or parts.netloc != host
		or parts.query
		or parts.fragment
		or not re.fullmatch(r"/[a-z0-9][a-z0-9-]{1,61}[a-z0-9]/?", parts.path)
		or "--" in parts.path
	):
		raise ValueError("storage_url must be https://ACCOUNT.blob.core.windows.net/CONTAINER")
	return value.rstrip("/")


def relative(value: str) -> str:
	if (
		not value
		or "\\" in value
		or "\x00" in value
		or unquote(value) != value
		or ":" in value
		or value.startswith("/")
		or any(p in ("", ".", "..") for p in value.split("/"))
	):
		raise ValueError("Expected a safe relative blob/archive path")
	return value


def record_path(options: dict) -> Path:
	identity = (
		options.get("tenant_id", "organizations") + ":" + options.get("client_id", DEFAULT_CLIENT)
	)
	base = Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))
	return base / "mise-azure-ac" / (hashlib.sha256(identity.encode()).hexdigest() + ".json")


def prompt(uri, code, expires):
	print(f"Azure sign-in: open {uri} and enter code {code} (expires {expires}).", file=sys.stderr)


def credential(options: dict):
	# Explicit modes prevent a failed CI identity from silently logging in as a person.
	mode = options.get("auth", "auto")
	if mode == "environment" or (mode == "auto" and os.getenv("AZURE_CLIENT_SECRET")):
		return EnvironmentCredential()
	if mode == "workload" or (mode == "auto" and os.getenv("AZURE_FEDERATED_TOKEN_FILE")):
		return WorkloadIdentityCredential()
	if mode == "managed":
		return ManagedIdentityCredential(client_id=options.get("managed_identity_client_id"))
	if mode not in ("auto", "device", "browser", "broker"):
		raise ValueError(
			"auth must be auto, broker, browser, device, environment, workload or managed"
		)
	path = record_path(options)
	record = AuthenticationRecord.deserialize(path.read_text()) if path.exists() else None
	kwargs = dict(
		tenant_id=options.get("tenant_id", "organizations"),
		client_id=options.get("client_id", DEFAULT_CLIENT),
		authentication_record=record,
		disable_automatic_authentication=True,
		timeout=300,
		cache_persistence_options=TokenCachePersistenceOptions(
			name="mise-azure-ac", allow_unencrypted_storage=False
		),
	)
	if mode == "broker" or (mode == "auto" and sys.platform == "win32"):
		if sys.platform != "win32":
			raise ValueError("auth=broker currently requires native Windows, not WSL")
		import msal
		from azure.identity.broker import InteractiveBrowserBrokerCredential

		return InteractiveBrowserBrokerCredential(
			**kwargs,
			parent_window_handle=msal.PublicClientApplication.CONSOLE_WINDOW_HANDLE,
			use_default_broker_account=True,
		)
	if mode == "device":
		return DeviceCodeCredential(**kwargs, prompt_callback=prompt)
	return InteractiveBrowserCredential(**kwargs)


def authenticate(options: dict):
	with credential(options) as cred:
		# Another install may have completed the login while we waited for the terminal.
		try:
			cred.get_token(SCOPE)
			return
		except AuthenticationRequiredError:
			if not isinstance(cred, (DeviceCodeCredential, InteractiveBrowserCredential)):
				raise
		if os.getenv("CI") or os.getenv("AZURE_AC_NONINTERACTIVE") == "1":
			raise ValueError("Interactive authentication disabled in CI/noninteractive mode")
		record = cred.authenticate(scopes=[SCOPE])
		path = record_path(options)
		path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
		# This file is an account selection record, not a token cache.
		fd, temporary = tempfile.mkstemp(dir=path.parent)
		try:
			with os.fdopen(fd, "w") as stream:
				stream.write(record.serialize())
			os.replace(temporary, path)
		finally:
			Path(temporary).unlink(missing_ok=True)


def read_json(container, name):
	return json.loads(container.download_blob(relative(name)).readall())


def versions(container, tool):
	entries = read_json(container, f"{relative(tool)}/index.json")["versions"]
	if not isinstance(entries, list) or not entries:
		raise ValueError("index.json needs a nonempty versions array, oldest first")
	seen = set()
	for entry in entries:
		version = entry if isinstance(entry, str) else entry["version"]
		relative(version)
		if "/" in version or version in seen:
			raise ValueError("Invalid or duplicate release version")
		seen.add(version)
	return {"versions": entries}


def checked_destination(root: Path, name: str) -> Path:
	parts = PurePosixPath(relative(name.rstrip("/"))).parts
	for part in parts:
		if (
			part.endswith((".", " "))
			or re.match(r"^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)", part, re.IGNORECASE)
			or any(ord(char) < 32 for char in part)
		):
			raise ValueError("Archive contains a nonportable Windows filename")
	return root.joinpath(*parts)


def extract(archive: Path, target: Path):
	"""Reject links, devices and traversal before extracting anything."""
	if archive.name.endswith(".zip"):
		with zipfile.ZipFile(archive) as source:
			for item in source.infolist():
				checked_destination(target, item.filename)
				mode = item.external_attr >> 16
				if stat.S_ISLNK(mode) or (
					stat.S_IFMT(mode) and not (stat.S_ISREG(mode) or stat.S_ISDIR(mode))
				):
					raise ValueError("Archive contains a link or special file")
			source.extractall(target)
			for item in source.infolist():
				path = checked_destination(target, item.filename)
				if not item.is_dir():
					path.chmod(0o755 if (item.external_attr >> 16) & 0o111 else 0o644)
	elif archive.name.endswith((".tar.gz", ".tar.xz", ".tar.bz2")):
		with tarfile.open(archive) as source:
			for item in source.getmembers():
				checked_destination(target, item.name)
				if not (item.isfile() or item.isdir()):
					raise ValueError("Archive contains a link or special file")
			source.extractall(target, filter="data")
	else:
		raise ValueError("Supported archives: .zip, .tar.gz, .tar.xz, .tar.bz2")


def install(container, request):
	tool, version = relative(request["tool"]), relative(request["version"])
	if "/" in version:
		raise ValueError("Version must be one path component")
	release = read_json(container, f"{tool}/{version}/release.json")
	asset = release["platforms"].get(request["platform"])
	if not asset:
		raise ValueError(f"Release has no artifact for {request['platform']}")
	digest = asset["sha256"]
	if not re.fullmatch(r"[a-fA-F0-9]{64}", digest):
		raise ValueError("Each artifact requires a SHA-256 digest")
	name = relative(asset["file"])
	if "/" in name:
		raise ValueError("Artifact file must be a filename, not a path")
	download = Path(request["download_path"])
	download.mkdir(parents=True, exist_ok=True)
	with tempfile.TemporaryDirectory(dir=download) as temporary:
		root = Path(temporary)
		archive = root / name
		checksum = hashlib.sha256()
		with archive.open("wb") as stream:
			for chunk in container.download_blob(f"{tool}/{version}/{name}").chunks():
				stream.write(chunk)
				checksum.update(chunk)
		if checksum.hexdigest() != digest.lower():
			raise ValueError("Artifact SHA-256 mismatch; nothing was installed")
		stage = root / "extracted"
		stage.mkdir()
		extract(archive, stage)
		if not (stage / "bin").is_dir():
			raise ValueError("Artifact must contain bin/ at the archive root")
		shutil.copytree(stage, Path(request["install_path"]), dirs_exist_ok=True)
	return {}


def list_blobs(container, prefix):
	return [
		{"name": blob.name, "size": blob.size}
		for blob in container.list_blobs(name_starts_with=prefix)
	]


def dispatch(request):
	options = request["options"]
	url = storage_url(options["storage_url"])
	if os.getenv("AZURE_AC_LOGIN") == "1":
		authenticate(options)
		return {}
	with credential(options) as cred:
		# Probe before any operation, so no prompt can disappear inside cmd.exec.
		cred.get_token(SCOPE)
		with ContainerClient.from_container_url(url, credential=cred) as container:
			action = request["action"]
			if action == "versions":
				return versions(container, request["tool"])
			if action == "install":
				return install(container, request)
			if action == "list":
				return list_blobs(container, request.get("prefix", ""))
			if action in ("read", "download"):
				blob = relative(request["blob"])
				data = container.download_blob(blob)
				if action == "read":
					return {"text": data.readall().decode("utf-8")}
				with Path(request["file"]).open("xb") as stream:
					data.readinto(stream)
				return {"downloaded": blob}
			if action == "upload":
				with Path(request["file"]).open("rb") as stream:
					container.upload_blob(
						relative(request["blob"]), stream, overwrite=request.get("overwrite", False)
					)
				return {"uploaded": request["blob"]}
			raise ValueError("Unknown action")


def main():
	if sys.argv[1:] == ["hook"]:
		request = json.loads(os.environ["AZURE_AC_REQUEST"])
		hook = True
	else:
		parser = argparse.ArgumentParser(description=__doc__)
		parser.add_argument("--storage-url", required=True)
		parser.add_argument("--tenant-id")
		parser.add_argument("--client-id")
		parser.add_argument("--auth", default="auto")
		commands = parser.add_subparsers(dest="action", required=True)
		commands.add_parser("login")
		commands.add_parser("list").add_argument("--prefix", default="")
		commands.add_parser("versions").add_argument("tool")
		for action in ("read", "download", "upload"):
			sub = commands.add_parser(action)
			sub.add_argument("blob")
			if action != "read":
				sub.add_argument("file")
			if action == "upload":
				sub.add_argument("--overwrite", action="store_true")
		args = vars(parser.parse_args())
		options = {
			key: args.pop(key)
			for key in ("storage_url", "tenant_id", "client_id", "auth")
			if args.get(key) is not None
		}
		request = {**args, "options": options}
		hook = False
		storage_url(options["storage_url"])
		if request["action"] == "login":
			authenticate(options)
			return
	try:
		result = dispatch(request)
	except AuthenticationRequiredError:
		if hook:
			result = {"authentication_required": True}
		else:
			authenticate(request["options"])
			result = dispatch(request)
	print(json.dumps(result))


if __name__ == "__main__":
	try:
		main()
	except HttpResponseError as exc:
		# Don't print service response bodies, URLs, headers or credential details.
		print(
			f"Azure request failed (HTTP {exc.status_code}, "
			f"type {type(exc).__name__}). Check authentication, tenant/client ID, "
			"Blob Data Reader/Contributor permissions and network access. "
			"Persistent login needs an OS keyring (Windows DPAPI, macOS Keychain, "
			"or Linux Secret Service); plaintext token storage is disabled.",
			file=sys.stderr,
		)
		sys.exit(1)
	except (ValueError, KeyError, OSError) as exc:
		print(f"azure-ac: {exc}", file=sys.stderr)
		sys.exit(1)

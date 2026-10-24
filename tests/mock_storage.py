"""Filesystem blob fake shared by unit tests and the subprocess-only mock runtime."""

import io
from pathlib import Path
from types import SimpleNamespace


class Download:
	def __init__(self, value):
		self.value = value

	def readall(self):
		return self.value

	def chunks(self):
		yield self.value

	def readinto(self, stream):
		return stream.write(self.value)


class Container:
	def __init__(self, root):
		self.root = Path(root)

	def __enter__(self):
		return self

	def __exit__(self, *args):
		pass

	def download_blob(self, name):
		return Download((self.root / name).read_bytes())

	def upload_blob(self, name, data, overwrite=False):
		target = self.root / name
		target.parent.mkdir(parents=True, exist_ok=True)
		if isinstance(data, str):
			data = data.encode()
		if isinstance(data, bytes):
			data = io.BytesIO(data)
		with target.open("wb" if overwrite else "xb") as stream:
			stream.write(data.read())

	def list_blobs(self, name_starts_with=""):
		for path in sorted(self.root.rglob("*")):
			if path.is_file():
				name = path.relative_to(self.root).as_posix()
				if name.startswith(name_starts_with):
					yield SimpleNamespace(name=name, size=path.stat().st_size)

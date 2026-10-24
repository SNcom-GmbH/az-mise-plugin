"""Injected ONLY by tests via PYTHONPATH; production has no mock/auth-bypass switch."""

import os
from pathlib import Path
from types import SimpleNamespace

import azure.identity
import azure.storage.blob
from mock_storage import Container


class Credential:
	def __init__(self, **kwargs):
		pass

	def __enter__(self):
		return self

	def __exit__(self, *args):
		pass

	def get_token(self, *args):
		if not Path(os.environ["MOCK_AZURE_LOGIN"]).exists():
			raise azure.identity.AuthenticationRequiredError(scopes=args)
		return SimpleNamespace(token="test-not-a-real-token", expires_on=9999999999)

	def authenticate(self, **kwargs):
		Path(os.environ["MOCK_AZURE_LOGIN"]).touch()
		return azure.identity.AuthenticationRecord(
			tenant_id="mock-tenant",
			client_id="04b07795-8ddb-461a-bbee-02f9e1bf7b46",
			authority="login.microsoftonline.com",
			home_account_id="mock-account",
			username="mock@example.invalid",
		)


if os.environ.get("MOCK_AZURE_STORAGE"):
	azure.identity.InteractiveBrowserCredential = Credential
	azure.identity.DeviceCodeCredential = Credential
	azure.storage.blob.ContainerClient.from_container_url = staticmethod(
		lambda *args, **kwargs: Container(os.environ["MOCK_AZURE_STORAGE"])
	)

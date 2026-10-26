use crate::{
	auth::{http_client, sdk_error},
	config::blob_path,
};
use anyhow::{Context, Result, ensure};
use azure_core::{
	credentials::TokenCredential,
	http::{Body, ClientOptions, StatusCode, Transport},
};
use azure_storage_blob::{
	BlobContainerClient, BlobContainerClientOptions,
	models::{
		BlobClientDownloadResult, BlobClientUploadOptions, BlobContainerClientListBlobsOptions,
	},
};
use futures::TryStreamExt;
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path, sync::Arc};
use url::Url;

pub const MAX_LIST_RESULTS: usize = 100_000;
const MAX_READ_BYTES: usize = 16 * 1024 * 1024;

pub struct VersionedBlob {
	pub bytes: Vec<u8>,
	pub etag: String,
}

#[derive(Debug)]
pub struct WriteConflict;

impl std::fmt::Display for WriteConflict {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("Conditional blob write failed")
	}
}
impl std::error::Error for WriteConflict {}

fn write_error(error: azure_core::Error) -> anyhow::Error {
	if let azure_core::error::ErrorKind::HttpResponse {
		status, error_code, ..
	} = error.kind()
		&& (*status == StatusCode::PreconditionFailed
			|| (*status == StatusCode::Conflict
				&& error_code.as_deref() == Some("BlobAlreadyExists")))
	{
		return anyhow::Error::new(WriteConflict).context(sdk_error(error));
	}
	sdk_error(error)
}

async fn read_body(response: BlobClientDownloadResult) -> Result<Vec<u8>> {
	if let Some(length) = response.properties.content_length {
		ensure!(
			length <= MAX_READ_BYTES as u64,
			"JSON/text blob exceeds 16 MiB; use download"
		);
	}
	let mut stream = response.body;
	let mut result = Vec::new();
	while let Some(chunk) = stream.try_next().await.map_err(sdk_error)? {
		ensure!(
			result.len() + chunk.len() <= MAX_READ_BYTES,
			"JSON/text blob exceeds 16 MiB; use download"
		);
		result.extend_from_slice(&chunk);
	}
	Ok(result)
}

#[async_trait::async_trait]
pub trait Storage: Send + Sync {
	async fn read(&self, blob: &str) -> Result<Vec<u8>>;
	async fn download(&self, blob: &str, file: &Path, max_bytes: Option<u64>) -> Result<()>;
	async fn upload(&self, blob: &str, file: &Path, overwrite: bool) -> Result<()>;
	async fn list(&self, prefix: &str, limit: usize) -> Result<Value>;
	/// Only an absent blob returns None; authentication and other errors must propagate.
	async fn read_versioned(&self, blob: &str) -> Result<Option<VersionedBlob>>;
	/// Replace the exact ETag read, or create an absent blob when etag is None.
	async fn write_metadata(&self, blob: &str, bytes: &[u8], etag: Option<&str>) -> Result<()>;
}

pub struct AzureStorage(BlobContainerClient);
impl AzureStorage {
	pub fn new(url: Url, credential: Arc<dyn TokenCredential>) -> Result<Self> {
		let config = BlobContainerClientOptions {
			client_options: ClientOptions {
				transport: Some(Transport::new(Arc::new(http_client()?))),
				..Default::default()
			},
			..Default::default()
		};
		Ok(Self(
			BlobContainerClient::new(url, Some(credential), Some(config)).map_err(sdk_error)?,
		))
	}
}

#[async_trait::async_trait]
impl Storage for AzureStorage {
	async fn read(&self, blob: &str) -> Result<Vec<u8>> {
		let response = self
			.0
			.blob_client(blob_path(blob)?)
			.download(None)
			.await
			.map_err(sdk_error)?;
		read_body(response).await
	}

	async fn read_versioned(&self, blob: &str) -> Result<Option<VersionedBlob>> {
		let response = match self.0.blob_client(blob_path(blob)?).download(None).await {
			Ok(response) => response,
			Err(error) => {
				if let azure_core::error::ErrorKind::HttpResponse {
					status, error_code, ..
				} = error.kind() && *status == StatusCode::NotFound
					&& error_code.as_deref() == Some("BlobNotFound")
				{
					return Ok(None);
				}
				return Err(sdk_error(error));
			}
		};
		let etag = response
			.properties
			.etag
			.as_ref()
			.context("Blob response has no ETag; refusing unsafe publication")?
			.to_string();
		let bytes = read_body(response).await?;
		Ok(Some(VersionedBlob { bytes, etag }))
	}

	async fn write_metadata(&self, blob: &str, bytes: &[u8], etag: Option<&str>) -> Result<()> {
		ensure!(bytes.len() <= MAX_READ_BYTES, "Metadata exceeds 16 MiB");
		let options = BlobClientUploadOptions {
			if_match: etag.map(Into::into),
			if_none_match: etag.is_none().then(|| "*".into()),
			blob_content_type: Some("application/json".into()),
			..Default::default()
		};
		self.0
			.blob_client(blob_path(blob)?)
			.upload(Body::from(bytes.to_vec()).into(), Some(options))
			.await
			.map_err(write_error)?;
		Ok(())
	}

	async fn download(&self, blob: &str, file: &Path, max_bytes: Option<u64>) -> Result<()> {
		let response = self
			.0
			.blob_client(blob_path(blob)?)
			.download(None)
			.await
			.map_err(sdk_error)?;
		if let (Some(max), Some(length)) = (max_bytes, response.properties.content_length) {
			ensure!(
				length <= max,
				"Download is {length} bytes, exceeding the {max}-byte limit"
			);
		}
		let mut stream = response.body;
		let mut output = fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(file)?;
		let result: Result<()> = async {
			let mut written = 0u64;
			while let Some(chunk) = stream.try_next().await.map_err(sdk_error)? {
				written = written
					.checked_add(chunk.len() as u64)
					.context("Download size overflow")?;
				if let Some(max) = max_bytes {
					ensure!(
						written <= max,
						"Download exceeds the {max}-byte limit; artifact rejected"
					);
				}
				output.write_all(&chunk)?;
			}
			output.sync_all()?;
			Ok(())
		}
		.await;
		drop(output);
		if result.is_err() {
			let _ = fs::remove_file(file);
		}
		result
	}

	async fn upload(&self, blob: &str, file: &Path, overwrite: bool) -> Result<()> {
		let stream = azure_storage_blob::stream::tokio::FileStream::builder(
			tokio::fs::File::open(file).await?,
		)
		.build()
		.await
		.map_err(sdk_error)?;
		let options = BlobClientUploadOptions {
			if_none_match: (!overwrite).then(|| "*".into()),
			..Default::default()
		};
		self.0
			.blob_client(blob_path(blob)?)
			.upload(Body::from(stream).into(), Some(options))
			.await
			.map_err(write_error)?;
		Ok(())
	}

	async fn list(&self, prefix: &str, limit: usize) -> Result<Value> {
		ensure!(
			(1..=MAX_LIST_RESULTS).contains(&limit),
			"limit must be between 1 and {MAX_LIST_RESULTS}"
		);
		let mut pager = self
			.0
			.list_blobs(Some(BlobContainerClientListBlobsOptions {
				prefix: Some(prefix.into()),
				..Default::default()
			}))
			.map_err(sdk_error)?;
		let mut result = Vec::new();
		while let Some(blob) = pager.try_next().await.map_err(sdk_error)? {
			ensure!(
				result.len() < limit,
				"Listing exceeds {limit} blobs; use a narrower prefix"
			);
			result.push(
				json!({"name": blob.name, "size": blob.properties.and_then(|p| p.content_length)}),
			);
		}
		Ok(json!(result))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use azure_core::{
		Bytes,
		http::{AsyncRawResponse, HttpClient, Request, StatusCode, headers::Headers},
	};
	use std::{collections::VecDeque, sync::Mutex};

	#[derive(Debug)]
	struct Http {
		responses: Mutex<VecDeque<(StatusCode, Headers, Bytes)>>,
		requests: Mutex<Vec<RecordedRequest>>,
	}
	type RecordedRequest = (String, Vec<(String, String)>);
	#[async_trait::async_trait]
	impl HttpClient for Http {
		async fn execute_request(&self, request: &Request) -> azure_core::Result<AsyncRawResponse> {
			assert_eq!(
				request.url().host_str(),
				Some("account.blob.core.windows.net")
			);
			let headers: Vec<_> = request
				.headers()
				.iter()
				.map(|(k, v)| (k.as_str().to_owned(), v.as_str().to_owned()))
				.collect();
			assert!(
				headers.iter().any(
					|(k, v)| k == "authorization" && v == "Bearer mock-token-not-a-real-secret"
				)
			);
			self.requests
				.lock()
				.expect("test mutex")
				.push((request.url().as_str().into(), headers));
			let (status, headers, body) = self
				.responses
				.lock()
				.expect("test mutex")
				.pop_front()
				.expect("Unexpected SDK request");
			Ok(AsyncRawResponse::from_bytes(status, headers, body))
		}
	}

	fn storage(responses: Vec<(StatusCode, Headers, Bytes)>) -> Result<(AzureStorage, Arc<Http>)> {
		let http = Arc::new(Http {
			responses: Mutex::new(responses.into()),
			requests: Mutex::new(Vec::new()),
		});
		let client = BlobContainerClient::new(
			Url::parse("https://account.blob.core.windows.net/tools")?,
			Some(Arc::new(crate::auth::StorageTokenCredential::new(
				"mock-token-not-a-real-secret".into(),
			)?)),
			Some(BlobContainerClientOptions {
				client_options: ClientOptions {
					transport: Some(Transport::new(http.clone())),
					..Default::default()
				},
				..Default::default()
			}),
		)
		.map_err(sdk_error)?;
		Ok((AzureStorage(client), http))
	}

	fn blob_response(body: Vec<u8>) -> (StatusCode, Headers, Bytes) {
		let mut headers = Headers::new();
		headers.insert("content-length", body.len().to_string());
		(StatusCode::Ok, headers, Bytes::from(body))
	}

	#[tokio::test]
	async fn supplied_token_authorizes_index_manifest_and_artifact() -> Result<()> {
		use sha2::{Digest, Sha256};
		use std::io::Cursor;
		let root = tempfile::tempdir()?;
		let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
		archive.start_file("bin/hello", zip::write::SimpleFileOptions::default())?;
		archive.write_all(b"hello")?;
		let bytes = archive.finish()?.into_inner();
		let manifest = json!({"platforms": {"linux-amd64": {
			"file": "hello.zip", "sha256": hex::encode(Sha256::digest(&bytes))
		}}})
		.to_string()
		.into_bytes();
		let (store, http) = storage(vec![
			blob_response(br#"{"versions":["1.0.0"]}"#.to_vec()),
			blob_response(manifest),
			blob_response(bytes),
		])?;
		let mut request: crate::config::Request = serde_json::from_value(json!({
			"action": "versions", "tool": "hello", "version": "1.0.0", "platform": "linux-amd64",
			"options": {"storage_url": "https://account.blob.core.windows.net/tools"},
			"install_path": root.path().join("install"), "download_path": root.path().join("downloads")
		}))?;
		assert_eq!(
			crate::dispatch(&store, &request).await?,
			json!({"versions": ["1.0.0"]})
		);
		request.action = "install".into();
		crate::dispatch(&store, &request).await?;
		assert_eq!(fs::read(root.path().join("install/bin/hello"))?, b"hello");
		let requests = http.requests.lock().expect("test mutex");
		assert_eq!(requests.len(), 3);
		for (request, blob) in requests.iter().zip([
			"hello/index.json",
			"hello/1.0.0/release.json",
			"hello/1.0.0/hello.zip",
		]) {
			assert!(
				Url::parse(&request.0)?
					.path()
					.replace("%2F", "/")
					.ends_with(blob)
			);
			assert!(!request.0.contains("mock-token"));
		}
		Ok(())
	}

	#[tokio::test]
	async fn rejected_token_fails_reads_and_downloads_without_identity_fallback() -> Result<()> {
		let root = tempfile::tempdir()?;
		for status in [StatusCode::Unauthorized, StatusCode::Forbidden] {
			for download in [false, true] {
				let mut headers = Headers::new();
				headers.insert("x-ms-error-code", "AuthenticationFailed");
				let (store, http) = storage(vec![(status, headers, Bytes::from_static(
					b"<Error><Code>AuthenticationFailed</Code><Message>mock-token-not-a-real-secret</Message></Error>"
				))])?;
				let target = root.path().join("artifact");
				let result = if download {
					store.download("hello/1.0.0/hello.zip", &target, None).await
				} else {
					store.read("hello/1.0.0/release.json").await.map(|_| ())
				};
				let error = result.expect_err("token rejected");
				assert!(!error.is::<crate::auth::AuthenticationRequired>());
				let message = format!("{error:#}");
				assert!(message.contains(&format!("HTTP {status}")));
				assert!(!message.contains("mock-token"));
				assert!(!target.exists());
				assert_eq!(http.requests.lock().expect("test mutex").len(), 1);
			}
		}
		Ok(())
	}

	#[tokio::test]
	async fn sdk_download_uses_bearer_auth_and_streams_bytes() -> Result<()> {
		let mut headers = Headers::new();
		headers.insert("content-length", "5");
		headers.insert("content-range", "bytes 0-4/5");
		let (store, _) = storage(vec![(
			StatusCode::PartialContent,
			headers,
			Bytes::from_static(b"hello"),
		)])?;
		assert_eq!(store.read("notes.txt").await?, b"hello");
		Ok(())
	}

	#[tokio::test]
	async fn sdk_download_rejects_oversized_artifacts() -> Result<()> {
		let root = tempfile::tempdir()?;
		let target = root.path().join("artifact");
		let mut headers = Headers::new();
		headers.insert("content-length", "5");
		headers.insert("content-range", "bytes 0-4/5");
		let (store, _) = storage(vec![(
			StatusCode::PartialContent,
			headers,
			Bytes::from_static(b"hello"),
		)])?;
		let error = store
			.download("hello/a", &target, Some(4))
			.await
			.expect_err("oversized download must fail");
		assert!(error.to_string().contains("exceed"));
		assert!(!target.exists());
		Ok(())
	}

	#[tokio::test]
	async fn sdk_upload_uses_conditional_writes_unless_explicitly_overwritten() -> Result<()> {
		let root = tempfile::tempdir()?;
		let file = root.path().join("file.txt");
		fs::write(&file, b"hello")?;
		let (store, http) = storage(vec![
			(StatusCode::Created, Headers::new(), Bytes::new()),
			(StatusCode::Created, Headers::new(), Bytes::new()),
		])?;
		store.upload("notes.txt", &file, false).await?;
		store.upload("notes.txt", &file, true).await?;
		let requests = http.requests.lock().expect("test mutex");
		assert!(
			requests[0]
				.1
				.iter()
				.any(|(k, v)| k == "if-none-match" && v == "*")
		);
		assert!(!requests[1].1.iter().any(|(k, _)| k == "if-none-match"));
		Ok(())
	}

	#[tokio::test]
	async fn supplied_token_authorizes_publication_in_artifact_manifest_index_order() -> Result<()>
	{
		let root = tempfile::tempdir()?;
		let file = root.path().join("hello");
		fs::write(&file, b"test binary")?;
		let missing = || {
			let mut headers = Headers::new();
			headers.insert("x-ms-error-code", "BlobNotFound");
			(StatusCode::NotFound, headers, Bytes::new())
		};
		let created = || (StatusCode::Created, Headers::new(), Bytes::new());
		let (store, http) = storage(vec![
			missing(),
			missing(),
			created(),
			missing(),
			created(),
			missing(),
			created(),
		])?;
		let request: crate::config::Request = serde_json::from_value(json!({
			"action": "publish", "tool": "hello", "version": "1.0.0",
			"platform": "linux-amd64", "file": file,
			"options": {"storage_url": "https://account.blob.core.windows.net/tools"}
		}))?;
		let result = crate::dispatch(&store, &request).await?;
		let artifact = format!(
			"hello/1.0.0/{}",
			result["artifact"]["file"].as_str().expect("filename")
		);
		let requests = http.requests.lock().expect("test mutex");
		assert_eq!(requests.len(), 7);
		for (request, blob) in requests.iter().zip([
			"hello/1.0.0/release.json",
			"hello/index.json",
			artifact.as_str(),
			"hello/1.0.0/release.json",
			"hello/1.0.0/release.json",
			"hello/index.json",
			"hello/index.json",
		]) {
			assert!(
				Url::parse(&request.0)?
					.path()
					.replace("%2F", "/")
					.ends_with(blob)
			);
		}
		for index in [2, 4, 6] {
			assert!(
				requests[index]
					.1
					.iter()
					.any(|(key, value)| key == "if-none-match" && value == "*")
			);
		}
		Ok(())
	}

	#[tokio::test]
	async fn existing_artifact_conflicts_remain_typed_and_actionable() -> Result<()> {
		let root = tempfile::tempdir()?;
		let file = root.path().join("hello");
		fs::write(&file, b"test binary")?;
		let mut headers = Headers::new();
		headers.insert("x-ms-error-code", "BlobAlreadyExists");
		let (store, _) = storage(vec![(StatusCode::Conflict, headers, Bytes::new())])?;
		let error = store
			.upload("hello/1.0.0/artifact.zip", &file, false)
			.await
			.expect_err("exists");
		assert!(error.is::<WriteConflict>());
		let message = format!("{error:#}");
		assert!(message.contains("HTTP 409"));
		assert!(message.contains("--overwrite"));
		Ok(())
	}

	#[tokio::test]
	async fn publication_reads_keep_etags_and_only_allow_missing_blobs() -> Result<()> {
		let mut headers = Headers::new();
		headers.insert("etag", "\"revision-1\"");
		let (store, _) = storage(vec![(StatusCode::Ok, headers, Bytes::from_static(b"{}"))])?;
		let snapshot = store
			.read_versioned("hello/index.json")
			.await?
			.expect("blob");
		assert_eq!(snapshot.bytes, b"{}");
		assert_eq!(snapshot.etag, "\"revision-1\"");
		for (status, code, missing) in [
			(StatusCode::NotFound, "BlobNotFound", true),
			(StatusCode::NotFound, "ContainerNotFound", false),
			(StatusCode::Unauthorized, "AuthenticationFailed", false),
			(
				StatusCode::Forbidden,
				"AuthorizationPermissionMismatch",
				false,
			),
		] {
			let mut headers = Headers::new();
			headers.insert("x-ms-error-code", code);
			let (store, http) = storage(vec![(status, headers, Bytes::new())])?;
			let result = store.read_versioned("hello/index.json").await;
			if missing {
				assert!(result?.is_none());
			} else {
				assert!(result.is_err());
			}
			assert_eq!(http.requests.lock().expect("test mutex").len(), 1);
		}
		let (store, _) = storage(vec![blob_response(b"{}".to_vec())])?;
		assert!(
			store.read_versioned("hello/index.json").await.is_err(),
			"Missing ETag must fail closed"
		);
		Ok(())
	}

	#[tokio::test]
	async fn publication_metadata_writes_use_etag_preconditions() -> Result<()> {
		let (store, http) = storage(vec![
			(StatusCode::Created, Headers::new(), Bytes::new()),
			(StatusCode::Created, Headers::new(), Bytes::new()),
			(StatusCode::PreconditionFailed, Headers::new(), Bytes::new()),
		])?;
		store
			.write_metadata("hello/index.json", b"{}", None)
			.await?;
		store
			.write_metadata("hello/index.json", b"{}", Some("\"revision-1\""))
			.await?;
		let error = store
			.write_metadata("hello/index.json", b"{}", Some("\"stale\""))
			.await
			.expect_err("conflict");
		assert!(error.is::<WriteConflict>());
		let requests = http.requests.lock().expect("test mutex");
		assert!(
			requests[0]
				.1
				.iter()
				.any(|(key, value)| key == "if-none-match" && value == "*")
		);
		assert!(!requests[0].1.iter().any(|(key, _)| key == "if-match"));
		assert!(
			requests[1]
				.1
				.iter()
				.any(|(key, value)| key == "if-match" && value == "\"revision-1\"")
		);
		assert!(!requests[1].1.iter().any(|(key, _)| key == "if-none-match"));
		Ok(())
	}

	#[tokio::test]
	async fn sdk_listing_follows_continuation_pages() -> Result<()> {
		let first = br#"<EnumerationResults ServiceEndpoint="https://account.blob.core.windows.net/" ContainerName="tools"><Blobs><Blob><Name>hello/a</Name><Properties><Content-Length>3</Content-Length><BlobType>BlockBlob</BlobType></Properties></Blob></Blobs><NextMarker>page-2</NextMarker></EnumerationResults>"#;
		let second = br#"<EnumerationResults ServiceEndpoint="https://account.blob.core.windows.net/" ContainerName="tools"><Blobs><Blob><Name>hello/b</Name><Properties><Content-Length>4</Content-Length><BlobType>BlockBlob</BlobType></Properties></Blob></Blobs><NextMarker /></EnumerationResults>"#;
		let (store, http) = storage(vec![
			(StatusCode::Ok, Headers::new(), Bytes::from_static(first)),
			(StatusCode::Ok, Headers::new(), Bytes::from_static(second)),
		])?;
		assert_eq!(
			store.list("hello/", MAX_LIST_RESULTS).await?,
			json!([{"name": "hello/a", "size": 3}, {"name": "hello/b", "size": 4}])
		);
		let requests = http.requests.lock().expect("test mutex");
		assert!(requests[0].0.contains("prefix=hello"));
		assert!(requests[1].0.contains("marker=page-2"));
		Ok(())
	}

	#[tokio::test]
	async fn sdk_listing_enforces_limit() -> Result<()> {
		let first = br#"<EnumerationResults ServiceEndpoint="https://account.blob.core.windows.net/" ContainerName="tools"><Blobs><Blob><Name>hello/a</Name><Properties><Content-Length>3</Content-Length><BlobType>BlockBlob</BlobType></Properties></Blob></Blobs><NextMarker>page-2</NextMarker></EnumerationResults>"#;
		let second = br#"<EnumerationResults ServiceEndpoint="https://account.blob.core.windows.net/" ContainerName="tools"><Blobs><Blob><Name>hello/b</Name><Properties><Content-Length>4</Content-Length><BlobType>BlockBlob</BlobType></Properties></Blob></Blobs><NextMarker /></EnumerationResults>"#;
		let (store, _) = storage(vec![
			(StatusCode::Ok, Headers::new(), Bytes::from_static(first)),
			(StatusCode::Ok, Headers::new(), Bytes::from_static(second)),
		])?;
		let error = store
			.list("hello/", 1)
			.await
			.expect_err("limit must be enforced");
		assert!(error.to_string().contains("exceeds"));
		Ok(())
	}
}

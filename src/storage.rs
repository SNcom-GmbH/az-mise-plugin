use crate::{
	auth::{http_client, sdk_error},
	config::relative,
};
use anyhow::{Result, ensure};
use azure_core::{
	credentials::TokenCredential,
	http::{Body, ClientOptions, Transport},
};
use azure_storage_blob::{
	BlobContainerClient, BlobContainerClientOptions,
	models::{BlobClientUploadOptions, BlobContainerClientListBlobsOptions},
};
use futures::TryStreamExt;
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path, sync::Arc};
use url::Url;

#[async_trait::async_trait]
pub trait Storage: Send + Sync {
	async fn read(&self, blob: &str) -> Result<Vec<u8>>;
	async fn download(&self, blob: &str, file: &Path) -> Result<()>;
	async fn upload(&self, blob: &str, file: &Path, overwrite: bool) -> Result<()>;
	async fn list(&self, prefix: &str) -> Result<Value>;
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
			.blob_client(relative(blob)?)
			.download(None)
			.await
			.map_err(sdk_error)?;
		let mut stream = response.body;
		let mut result = Vec::new();
		while let Some(chunk) = stream.try_next().await.map_err(sdk_error)? {
			ensure!(
				result.len() + chunk.len() <= 16 * 1024 * 1024,
				"JSON/text blob exceeds 16 MiB; use download"
			);
			result.extend_from_slice(&chunk);
		}
		Ok(result)
	}

	async fn download(&self, blob: &str, file: &Path) -> Result<()> {
		let response = self
			.0
			.blob_client(relative(blob)?)
			.download(None)
			.await
			.map_err(sdk_error)?;
		let mut stream = response.body;
		let mut output = fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(file)?;
		let result: Result<()> = async {
			while let Some(chunk) = stream.try_next().await.map_err(sdk_error)? {
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
			.blob_client(relative(blob)?)
			.upload(Body::from(stream).into(), Some(options))
			.await
			.map_err(sdk_error)?;
		Ok(())
	}

	async fn list(&self, prefix: &str) -> Result<Value> {
		let mut pager = self
			.0
			.list_blobs(Some(BlobContainerClientListBlobsOptions {
				prefix: Some(prefix.into()),
				..Default::default()
			}))
			.map_err(sdk_error)?;
		let mut result = Vec::new();
		while let Some(blob) = pager.try_next().await.map_err(sdk_error)? {
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
		credentials::{AccessToken, TokenRequestOptions},
		http::{AsyncRawResponse, HttpClient, Request, StatusCode, headers::Headers},
		time::{Duration, OffsetDateTime},
	};
	use std::{collections::VecDeque, sync::Mutex};

	#[derive(Debug)]
	struct Credential;
	#[async_trait::async_trait]
	impl TokenCredential for Credential {
		async fn get_token(
			&self,
			scopes: &[&str],
			_: Option<TokenRequestOptions<'_>>,
		) -> azure_core::Result<AccessToken> {
			assert_eq!(scopes, [crate::config::SCOPE]);
			Ok(AccessToken::new(
				"mock-token-not-a-real-secret",
				OffsetDateTime::now_utc() + Duration::hours(1),
			))
		}
	}

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
			Some(Arc::new(Credential)),
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
	async fn sdk_listing_follows_continuation_pages() -> Result<()> {
		let first = br#"<EnumerationResults ServiceEndpoint="https://account.blob.core.windows.net/" ContainerName="tools"><Blobs><Blob><Name>hello/a</Name><Properties><Content-Length>3</Content-Length><BlobType>BlockBlob</BlobType></Properties></Blob></Blobs><NextMarker>page-2</NextMarker></EnumerationResults>"#;
		let second = br#"<EnumerationResults ServiceEndpoint="https://account.blob.core.windows.net/" ContainerName="tools"><Blobs><Blob><Name>hello/b</Name><Properties><Content-Length>4</Content-Length><BlobType>BlockBlob</BlobType></Properties></Blob></Blobs><NextMarker /></EnumerationResults>"#;
		let (store, http) = storage(vec![
			(StatusCode::Ok, Headers::new(), Bytes::from_static(first)),
			(StatusCode::Ok, Headers::new(), Bytes::from_static(second)),
		])?;
		assert_eq!(
			store.list("hello/").await?,
			json!([{"name": "hello/a", "size": 3}, {"name": "hello/b", "size": 4}])
		);
		let requests = http.requests.lock().expect("test mutex");
		assert!(requests[0].0.contains("prefix=hello"));
		assert!(requests[1].0.contains("marker=page-2"));
		Ok(())
	}
}

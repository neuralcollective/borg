use std::path::Path;

use anyhow::{anyhow, Context, Result};
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_s3::{primitives::ByteStream, Client};
use borg_core::config::Config;

#[derive(Clone)]
pub enum FileStorage {
    Local {
        data_dir: String,
    },
    S3 {
        bucket: String,
        prefix: String,
        client: Client,
    },
}

impl FileStorage {
    pub async fn from_config(config: &Config) -> Result<Self> {
        if config.storage_backend.trim().eq_ignore_ascii_case("s3") {
            if config.s3_bucket.trim().is_empty() {
                return Err(anyhow!("S3 storage backend selected but S3_BUCKET/s3_bucket is empty"));
            }
            let mut loader = aws_config::defaults(BehaviorVersion::latest())
                .region(Region::new(config.s3_region.clone()));
            if !config.s3_access_key.is_empty() && !config.s3_secret_key.is_empty() {
                loader = loader.credentials_provider(Credentials::new(
                    config.s3_access_key.clone(),
                    config.s3_secret_key.clone(),
                    None,
                    None,
                    "borg-server-config",
                ));
            }
            let shared = loader.load().await;
            let mut s3_builder = aws_sdk_s3::config::Builder::from(&shared);
            if !config.s3_endpoint.trim().is_empty() {
                s3_builder = s3_builder
                    .endpoint_url(config.s3_endpoint.clone())
                    .force_path_style(true);
            }
            let client = Client::from_conf(s3_builder.build());
            let mut prefix = config.s3_prefix.trim().to_string();
            if !prefix.is_empty() && !prefix.ends_with('/') {
                prefix.push('/');
            }
            return Ok(Self::S3 {
                bucket: config.s3_bucket.clone(),
                prefix,
                client,
            });
        }

        Ok(Self::Local {
            data_dir: config.data_dir.clone(),
        })
    }

    fn parse_s3_uri(uri: &str) -> Option<(String, String)> {
        let rest = uri.strip_prefix("s3://")?;
        let (bucket, key) = rest.split_once('/')?;
        Some((bucket.to_string(), key.to_string()))
    }

    pub async fn put_project_file(
        &self,
        project_id: i64,
        object_name: &str,
        bytes: &[u8],
    ) -> Result<String> {
        match self {
            Self::Local { data_dir } => {
                let files_dir = format!("{data_dir}/projects/{project_id}/files");
                tokio::fs::create_dir_all(&files_dir)
                    .await
                    .with_context(|| format!("create project dir {files_dir}"))?;
                let stored_path = format!("{files_dir}/{object_name}");
                tokio::fs::write(&stored_path, bytes)
                    .await
                    .with_context(|| format!("write project file {stored_path}"))?;
                Ok(stored_path)
            }
            Self::S3 {
                bucket,
                prefix,
                client,
            } => {
                let key = format!("{prefix}projects/{project_id}/files/{object_name}");
                client
                    .put_object()
                    .bucket(bucket)
                    .key(&key)
                    .body(ByteStream::from(bytes.to_vec()))
                    .send()
                    .await
                    .context("s3 put_object project file")?;
                Ok(format!("s3://{bucket}/{key}"))
            }
        }
    }

    pub async fn put_project_file_from_path(
        &self,
        project_id: i64,
        object_name: &str,
        source_path: &str,
    ) -> Result<String> {
        match self {
            Self::Local { data_dir } => {
                let files_dir = format!("{data_dir}/projects/{project_id}/files");
                tokio::fs::create_dir_all(&files_dir)
                    .await
                    .with_context(|| format!("create project dir {files_dir}"))?;
                let stored_path = format!("{files_dir}/{object_name}");
                tokio::fs::copy(source_path, &stored_path)
                    .await
                    .with_context(|| format!("copy project file {source_path} -> {stored_path}"))?;
                Ok(stored_path)
            }
            Self::S3 {
                bucket,
                prefix,
                client,
            } => {
                let key = format!("{prefix}projects/{project_id}/files/{object_name}");
                let body = ByteStream::from_path(std::path::Path::new(source_path))
                    .await
                    .context("create s3 bytestream from path")?;
                client
                    .put_object()
                    .bucket(bucket)
                    .key(&key)
                    .body(body)
                    .send()
                    .await
                    .context("s3 put_object project file from path")?;
                Ok(format!("s3://{bucket}/{key}"))
            }
        }
    }

    pub async fn read_all(&self, stored_path: &str) -> Result<Vec<u8>> {
        match self {
            Self::Local { .. } => tokio::fs::read(stored_path)
                .await
                .with_context(|| format!("read local file {stored_path}")),
            Self::S3 { client, .. } => {
                if let Some((bucket, key)) = Self::parse_s3_uri(stored_path) {
                    let out = client
                        .get_object()
                        .bucket(bucket)
                        .key(key)
                        .send()
                        .await
                        .context("s3 get_object")?;
                    let data = out
                        .body
                        .collect()
                        .await
                        .context("s3 collect object body")?;
                    Ok(data.into_bytes().to_vec())
                } else if Path::new(stored_path).exists() {
                    tokio::fs::read(stored_path)
                        .await
                        .with_context(|| format!("read fallback local file {stored_path}"))
                } else {
                    Err(anyhow!("unsupported stored path for S3 backend: {stored_path}"))
                }
            }
        }
    }
}

//! Whisper model catalog and the resumable, verified downloader.

use crate::settings::ModelId;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: ModelId,
    pub name: &'static str,
    pub description: &'static str,
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub english_only: bool,
}

pub const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: ModelId::Fast,
        name: "Fast",
        description: "Quick and light. English only.",
        file: "ggml-base.en-q5_1.bin",
        size: 59_721_011,
        sha256: "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f",
        english_only: true,
    },
    ModelInfo {
        id: ModelId::Accurate,
        name: "Accurate",
        description: "Better with names and accents. English only. Slower.",
        file: "ggml-small.en.bin",
        size: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        english_only: true,
    },
    ModelInfo {
        id: ModelId::Multilingual,
        name: "Multilingual",
        description: "Understands many languages.",
        file: "ggml-base.bin",
        size: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        english_only: false,
    },
];

pub fn info(id: ModelId) -> &'static ModelInfo {
    MODELS
        .iter()
        .find(|m| m.id == id)
        .expect("every ModelId is in the catalog")
}

pub fn url(m: &ModelInfo) -> String {
    format!("{BASE_URL}{}", m.file)
}

/// Language actually passed to Whisper: English-only models always get "en".
pub fn effective_language(id: ModelId, chosen: &str) -> String {
    if info(id).english_only {
        "en".into()
    } else {
        chosen.to_string()
    }
}

pub fn human_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{} MB", (bytes as f64 / 1e6).round() as u64)
    }
}

/// Verifies a file's SHA-256.
pub fn verify_file(path: &std::path::Path, expected_hex: &str) -> std::io::Result<bool> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok(hex.eq_ignore_ascii_case(expected_hex))
}

#[cfg(feature = "net")]
pub mod download {
    use super::*;
    use futures_util::StreamExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tokio::io::AsyncWriteExt;

    #[derive(Debug, thiserror::Error)]
    pub enum DownloadError {
        #[error("Download cancelled")]
        Cancelled,
        #[error("No internet connection")]
        Network(String),
        #[error("The server said {0}")]
        Http(u16),
        #[error("The download was damaged. Please try again.")]
        Verify,
        #[error("Couldn't save the file: {0}")]
        Io(String),
    }

    pub fn part_path(dir: &Path, m: &ModelInfo) -> PathBuf {
        dir.join(format!("{}.part", m.file))
    }

    /// Downloads `m` into `dir`, resuming any partial file, verifying the
    /// SHA-256, then atomically renaming into place. `progress(done, total)`.
    pub async fn download(
        client: &reqwest::Client,
        m: &ModelInfo,
        dir: &Path,
        cancel: Arc<AtomicBool>,
        mut progress: impl FnMut(u64, u64) + Send,
    ) -> Result<PathBuf, DownloadError> {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| DownloadError::Io(e.to_string()))?;
        let final_path = dir.join(m.file);
        let part = part_path(dir, m);
        let mut have = tokio::fs::metadata(&part).await.map(|md| md.len()).unwrap_or(0);
        if have > m.size {
            let _ = tokio::fs::remove_file(&part).await;
            have = 0;
        }
        if have < m.size {
            let mut req = client.get(url(m));
            if have > 0 {
                req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
            }
            let resp = req.send().await.map_err(|e| DownloadError::Network(e.to_string()))?;
            let status = resp.status().as_u16();
            let resumed = status == 206;
            if !(resumed || status == 200) {
                return Err(DownloadError::Http(status));
            }
            if !resumed {
                have = 0;
            }
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(resumed)
                .truncate(!resumed)
                .open(&part)
                .await
                .map_err(|e| DownloadError::Io(e.to_string()))?;
            let mut stream = resp.bytes_stream();
            progress(have, m.size);
            while let Some(chunk) = stream.next().await {
                if cancel.load(Ordering::Relaxed) {
                    let _ = file.flush().await;
                    return Err(DownloadError::Cancelled);
                }
                let chunk = chunk.map_err(|e| DownloadError::Network(e.to_string()))?;
                file.write_all(&chunk)
                    .await
                    .map_err(|e| DownloadError::Io(e.to_string()))?;
                have += chunk.len() as u64;
                progress(have.min(m.size), m.size);
            }
            file.flush().await.map_err(|e| DownloadError::Io(e.to_string()))?;
        }
        let part2 = part.clone();
        let expected = m.sha256;
        let ok = tokio::task::spawn_blocking(move || verify_file(&part2, expected))
            .await
            .map_err(|e| DownloadError::Io(e.to_string()))?
            .map_err(|e| DownloadError::Io(e.to_string()))?;
        if !ok {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(DownloadError::Verify);
        }
        tokio::fs::rename(&part, &final_path)
            .await
            .map_err(|e| DownloadError::Io(e.to_string()))?;
        Ok(final_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_complete() {
        assert_eq!(info(ModelId::Fast).file, "ggml-base.en-q5_1.bin");
        assert_eq!(info(ModelId::Accurate).file, "ggml-small.en.bin");
        assert_eq!(info(ModelId::Multilingual).file, "ggml-base.bin");
        for m in MODELS {
            assert_eq!(m.sha256.len(), 64);
            assert!(url(m).starts_with("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/"));
        }
    }

    #[test]
    fn english_models_force_english() {
        assert_eq!(effective_language(ModelId::Fast, "de"), "en");
        assert_eq!(effective_language(ModelId::Multilingual, "de"), "de");
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(59_721_011), "60 MB");
        assert_eq!(human_size(1_500_000_000), "1.5 GB");
    }

    #[test]
    fn verify_detects_mismatch() {
        let dir = std::env::temp_dir().join(format!("wirl-verify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.bin");
        std::fs::write(&p, b"abc").unwrap();
        assert!(verify_file(&p, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad").unwrap());
        assert!(!verify_file(&p, &"0".repeat(64)).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }
}

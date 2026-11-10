//! Speech model catalog and the resumable, verified downloader.

use crate::settings::ModelId;
use serde::Serialize;

/// Which speech engine (and so which sidecar program) runs a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Engine {
    /// whisper.cpp (`wirl-whisper`).
    Whisper,
    /// SenseVoice through sherpa-onnx (`wirl-sensevoice`).
    SenseVoice,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: ModelId,
    pub name: &'static str,
    pub description: &'static str,
    /// What sits in the models folder when the model is ready: a file
    /// (Whisper) or a folder unpacked from `archive` (SenseVoice).
    pub file: &'static str,
    /// The `.tar.bz2` that is downloaded and unpacked into `file`, if any.
    pub archive: Option<&'static str>,
    /// Download size in bytes (of the archive when there is one).
    pub size: u64,
    /// SHA-256 of what is downloaded.
    pub sha256: &'static str,
    pub english_only: bool,
    pub engine: Engine,
}

pub const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";
/// Official sherpa-onnx ASR model releases (k2-fsa/sherpa-onnx, tag asr-models).
pub const SHERPA_MODELS_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/";

/// SenseVoice's language for the Cantonese model. Tested against "auto" on the
/// model's Cantonese clips and on Cantonese mixed with English: the results were
/// the same or slightly better, and short clips can't be mistaken for Mandarin.
pub const CANTONESE_LANGUAGE: &str = "yue";

/// Files the SenseVoice sidecar needs from the unpacked archive.
pub const SENSEVOICE_FILES: &[&str] = &["model.int8.onnx", "tokens.txt"];

pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: ModelId::Fast,
        name: "Fast",
        description: "Quick and light. English only.",
        file: "ggml-base.en-q5_1.bin",
        size: 59_721_011,
        sha256: "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f",
        english_only: true,
        archive: None,
        engine: Engine::Whisper,
    },
    ModelInfo {
        id: ModelId::Accurate,
        name: "Accurate",
        description: "Better with names and accents. English only. Slower.",
        file: "ggml-small.en.bin",
        size: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        english_only: true,
        archive: None,
        engine: Engine::Whisper,
    },
    ModelInfo {
        id: ModelId::Multilingual,
        name: "Multilingual",
        description: "Understands many languages.",
        file: "ggml-base.bin",
        size: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        english_only: false,
        archive: None,
        engine: Engine::Whisper,
    },
    ModelInfo {
        id: ModelId::Cantonese,
        name: "Cantonese (粵語)",
        description: "Cantonese with English mixed in. Writes as you speak, in traditional characters.",
        file: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09",
        archive: Some("sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2"),
        size: 165_783_878,
        sha256: "7305f7905bfcf77fa0b39388a313f3da35c68d971661a65475b56fb2162c8e63",
        english_only: false,
        engine: Engine::SenseVoice,
    },
];

pub fn info(id: ModelId) -> &'static ModelInfo {
    MODELS
        .iter()
        .find(|m| m.id == id)
        .expect("every ModelId is in the catalog")
}

pub fn url(m: &ModelInfo) -> String {
    match m.archive {
        Some(a) => format!("{SHERPA_MODELS_URL}{a}"),
        None => format!("{BASE_URL}{}", m.file),
    }
}

/// The name of what is downloaded (the archive, or the model file itself).
pub fn download_name(m: &ModelInfo) -> &'static str {
    m.archive.unwrap_or(m.file)
}

/// Language actually passed to the engine: English-only models always get
/// "en", the Cantonese model always gets [`CANTONESE_LANGUAGE`].
pub fn effective_language(id: ModelId, chosen: &str) -> String {
    let m = info(id);
    if m.engine == Engine::SenseVoice {
        CANTONESE_LANGUAGE.into()
    } else if m.english_only {
        "en".into()
    } else {
        chosen.to_string()
    }
}

/// Whether the model has a fixed language (the Language picker is disabled).
pub fn fixed_language(id: ModelId) -> bool {
    let m = info(id);
    m.english_only || m.engine == Engine::SenseVoice
}

/// "Fast · English (US)", or just the model name when the language is part of it.
pub fn summary(id: ModelId, chosen: &str) -> String {
    let m = info(id);
    if m.engine == Engine::SenseVoice {
        m.name.to_string()
    } else {
        format!(
            "{} · {}",
            m.name,
            crate::settings::language_label(&effective_language(id, chosen))
        )
    }
}

/// Unpacks the files a SenseVoice model needs from its `.tar.bz2` into
/// `dest` (flat). Anything else in the archive (test audio, README) is skipped.
pub fn unpack_sensevoice(archive: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    std::fs::create_dir_all(dest)?;
    let f = std::io::BufReader::new(std::fs::File::open(archive)?);
    let mut tar = tar::Archive::new(bzip2::read::MultiBzDecoder::new(f));
    let mut found = 0;
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Only top-level files of the model folder: "<folder>/<name>".
        if path.components().count() != 2 || !SENSEVOICE_FILES.contains(&name) {
            continue;
        }
        let mut out = std::fs::File::create(dest.join(name))?;
        std::io::copy(&mut entry, &mut out)?;
        out.sync_all()?;
        found += 1;
    }
    if found != SENSEVOICE_FILES.len() {
        return Err(Error::new(ErrorKind::InvalidData, "the model archive is missing files"));
    }
    Ok(())
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
        dir.join(format!("{}.part", download_name(m)))
    }

    /// Downloads `m` into `dir`, resuming any partial file, verifying the
    /// SHA-256, then atomically renaming into place. `progress(done, total)`.
    /// An archive is then unpacked into its folder (`unpacking()` is called
    /// first, since that takes a little while) and deleted.
    pub async fn download(
        client: &reqwest::Client,
        m: &ModelInfo,
        dir: &Path,
        cancel: Arc<AtomicBool>,
        mut progress: impl FnMut(u64, u64) + Send,
        unpacking: impl FnOnce() + Send,
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
        if m.archive.is_some() {
            unpacking();
            // Unpack next to the final folder, then rename, so a half-unpacked
            // model never looks ready.
            let tmp = dir.join(format!("{}.unpack", m.file));
            let (part2, tmp2, final2) = (part.clone(), tmp.clone(), final_path.clone());
            let r = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
                let _ = std::fs::remove_dir_all(&tmp2);
                unpack_sensevoice(&part2, &tmp2)?;
                let _ = std::fs::remove_dir_all(&final2);
                std::fs::rename(&tmp2, &final2)
            })
            .await
            .map_err(|e| DownloadError::Io(e.to_string()))?;
            let _ = tokio::fs::remove_file(&part).await;
            if let Err(e) = r {
                let _ = tokio::fs::remove_dir_all(&tmp).await;
                return Err(DownloadError::Io(e.to_string()));
            }
            return Ok(final_path);
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
            match m.engine {
                Engine::Whisper => {
                    assert!(url(m).starts_with("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/"))
                }
                Engine::SenseVoice => assert!(m.archive.is_some()),
            }
        }
    }

    #[test]
    fn cantonese_model_is_the_official_sensevoice_release() {
        let m = info(ModelId::Cantonese);
        assert_eq!(m.name, "Cantonese (粵語)");
        assert_eq!(
            m.description,
            "Cantonese with English mixed in. Writes as you speak, in traditional characters."
        );
        assert_eq!(m.engine, Engine::SenseVoice);
        assert_eq!(
            url(m),
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2"
        );
        assert_eq!(human_size(m.size), "166 MB");
    }

    #[test]
    fn english_models_force_english() {
        assert_eq!(effective_language(ModelId::Fast, "de"), "en");
        assert_eq!(effective_language(ModelId::Multilingual, "de"), "de");
        assert_eq!(effective_language(ModelId::Cantonese, "en"), "yue");
        assert!(fixed_language(ModelId::Cantonese) && fixed_language(ModelId::Fast));
        assert!(!fixed_language(ModelId::Multilingual));
        assert_eq!(summary(ModelId::Fast, "de"), "Fast · English (US)");
        assert_eq!(summary(ModelId::Cantonese, "en"), "Cantonese (粵語)");
    }

    #[test]
    fn unpacks_only_the_model_files() {
        let dir = std::env::temp_dir().join(format!("wirl-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("m.tar.bz2");
        {
            let f = std::fs::File::create(&archive).unwrap();
            let bz = bzip2::write::BzEncoder::new(f, bzip2::Compression::fast());
            let mut t = tar::Builder::new(bz);
            for (name, body) in [
                ("m/model.int8.onnx", &b"onnx"[..]),
                ("m/tokens.txt", b"tok"),
                ("m/test_wavs/yue.wav", b"wav"),
                ("m/README.md", b"hi"),
            ] {
                let mut h = tar::Header::new_gnu();
                h.set_size(body.len() as u64);
                h.set_mode(0o644);
                h.set_cksum();
                t.append_data(&mut h, name, body).unwrap();
            }
            t.into_inner().unwrap().finish().unwrap();
        }
        let out = dir.join("out");
        unpack_sensevoice(&archive, &out).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["model.int8.onnx", "tokens.txt"]);
        assert_eq!(std::fs::read(out.join("tokens.txt")).unwrap(), b"tok");
        // An archive without the model is rejected.
        let bad = dir.join("bad.tar.bz2");
        {
            let f = std::fs::File::create(&bad).unwrap();
            let bz = bzip2::write::BzEncoder::new(f, bzip2::Compression::fast());
            let mut t = tar::Builder::new(bz);
            let mut h = tar::Header::new_gnu();
            h.set_size(1);
            h.set_cksum();
            t.append_data(&mut h, "m/tokens.txt", &b"t"[..]).unwrap();
            t.into_inner().unwrap().finish().unwrap();
        }
        assert!(unpack_sensevoice(&bad, &dir.join("out2")).is_err());
        let _ = std::fs::remove_dir_all(dir);
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

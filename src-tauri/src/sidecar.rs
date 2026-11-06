//! Runs Whisper in a separate process. If it crashes or runs out of memory
//! the app keeps going: pending requests fail, the reason is logged, and the
//! sidecar is restarted with the same model loaded.
//!
//! Two builds ship: `wirl-whisper` (Vulkan GPU, falls back to CPU inside
//! whisper.cpp when no GPU) and `wirl-whisper-cpu` (no Vulkan dependency).
//! If the GPU build can't start or load a model, the CPU build is used.

use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use wirl_core::ipc::{encode_pcm, Request, Response};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<(String, u64), String>>>>>;
type RestartHook = Arc<dyn Fn(String) + Send + Sync>;
type LoadWaiter = Arc<Mutex<Option<oneshot::Sender<Result<(bool, u64), String>>>>>;

struct Proc {
    stdin: ChildStdin,
    child: Child,
    generation: u64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarStatus {
    pub running: bool,
    pub model_path: Option<String>,
    pub gpu: bool,
    pub cpu_only: bool,
    pub restarts: u32,
    pub last_error: Option<String>,
}

pub struct Sidecar {
    proc_: AsyncMutex<Option<Proc>>,
    pending: Pending,
    load_waiter: LoadWaiter,
    next_id: AtomicU64,
    generation: AtomicU64,
    cpu_only: AtomicBool,
    want_model: Mutex<Option<String>>,
    status: Arc<Mutex<SidecarStatus>>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    on_restart: Mutex<Option<RestartHook>>,
    shutting_down: AtomicBool,
}

fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

/// Bundled sidecars sit next to the main executable.
fn binary_path(cpu_only: bool) -> Option<PathBuf> {
    let names: &[&str] = if cpu_only {
        &["wirl-whisper-cpu"]
    } else {
        &["wirl-whisper", "wirl-whisper-cpu"]
    };
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for n in names {
        let p = dir.join(exe_name(n));
        if p.exists() {
            return Some(p);
        }
    }
    None
}

impl Sidecar {
    pub fn new() -> Arc<Self> {
        Arc::new(Sidecar {
            proc_: AsyncMutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            load_waiter: Arc::new(Mutex::new(None)),
            next_id: AtomicU64::new(1),
            generation: AtomicU64::new(0),
            cpu_only: AtomicBool::new(false),
            want_model: Mutex::new(None),
            status: Arc::new(Mutex::new(SidecarStatus::default())),
            stderr_tail: Arc::new(Mutex::new(VecDeque::new())),
            on_restart: Mutex::new(None),
            shutting_down: AtomicBool::new(false),
        })
    }

    pub fn set_restart_hook(&self, f: impl Fn(String) + Send + Sync + 'static) {
        *self.on_restart.lock() = Some(Arc::new(f));
    }

    pub fn status(&self) -> SidecarStatus {
        let mut s = self.status.lock().clone();
        s.cpu_only = self.cpu_only.load(Ordering::Relaxed);
        s
    }

    pub fn is_loaded(&self, path: &str) -> bool {
        let s = self.status.lock();
        s.running && s.model_path.as_deref() == Some(path)
    }

    async fn spawn(self: &Arc<Self>) -> Result<(), String> {
        let cpu_only = self.cpu_only.load(Ordering::Relaxed);
        let path = binary_path(cpu_only).ok_or("Speech engine is missing. Please reinstall Wirl.")?;
        let mut cmd = Command::new(&path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| format!("start speech engine: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut st = self.status.lock();
            st.running = true;
            st.model_path = None;
            st.gpu = false;
        }
        // stderr tail for crash reports.
        let tail = self.stderr_tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let mut t = tail.lock();
                t.push_back(l);
                while t.len() > 30 {
                    t.pop_front();
                }
            }
        });
        // stdout dispatcher; EOF means the process died.
        let me = self.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let Ok(resp) = serde_json::from_str::<Response>(&l) else {
                    continue;
                };
                me.dispatch(resp);
            }
            me.on_exit(generation).await;
        });
        *self.proc_.lock().await = Some(Proc {
            stdin,
            child,
            generation,
        });
        log::info!("speech engine started: {}", path.display());
        Ok(())
    }

    fn dispatch(&self, r: Response) {
        match r {
            Response::Result { id, text, ms } => {
                if let Some(tx) = self.pending.lock().remove(&id) {
                    let _ = tx.send(Ok((text, ms)));
                }
            }
            Response::Error { id: Some(id), message } => {
                if let Some(tx) = self.pending.lock().remove(&id) {
                    let _ = tx.send(Err(message));
                }
            }
            Response::Error { id: None, message } => {
                if let Some(tx) = self.load_waiter.lock().take() {
                    let _ = tx.send(Err(message));
                }
            }
            Response::Loaded { model_path, gpu, ms } => {
                {
                    let mut st = self.status.lock();
                    st.model_path = Some(model_path);
                    st.gpu = gpu;
                }
                if let Some(tx) = self.load_waiter.lock().take() {
                    let _ = tx.send(Ok((gpu, ms)));
                }
            }
            Response::Ready { .. } | Response::Pong { .. } => {}
        }
    }

    async fn on_exit(self: Arc<Self>, generation: u64) {
        let mut guard = self.proc_.lock().await;
        let code = match guard.as_mut() {
            Some(p) if p.generation == generation => p.child.wait().await.ok().map(|s| s.to_string()),
            _ => return, // already replaced (deliberate restart)
        };
        *guard = None;
        drop(guard);
        {
            let mut st = self.status.lock();
            st.running = false;
            st.model_path = None;
        }
        for (_, tx) in self.pending.lock().drain() {
            let _ = tx.send(Err("Speech engine stopped".into()));
        }
        if let Some(tx) = self.load_waiter.lock().take() {
            let _ = tx.send(Err("Speech engine stopped while loading".into()));
        }
        if self.shutting_down.load(Ordering::Relaxed) {
            return;
        }
        let tail: Vec<String> = self.stderr_tail.lock().iter().rev().take(5).rev().cloned().collect();
        let reason = format!(
            "Speech engine exited ({}). {}",
            code.unwrap_or_else(|| "unknown status".into()),
            tail.join(" | ")
        );
        log::error!("{reason}");
        self.status.lock().restarts += 1;
        self.status.lock().last_error = Some(reason.clone());
        let hook = self.on_restart.lock().clone();
        if let Some(h) = hook {
            h(reason);
        }
        // Restart and reload, with a small delay to avoid a tight crash loop.
        tokio::spawn(self.clone().reload_later(Duration::from_millis(500)));
    }

    /// Boxed so the restart path (spawn -> exit -> reload -> spawn) has a
    /// nameable, Send future type.
    fn reload_later(self: Arc<Self>, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            let model = self.want_model.lock().clone();
            if let Some(model) = model {
                if let Err(e) = self.load(&model).await {
                    log::error!("reload after restart failed: {e}");
                }
            }
        })
    }

    async fn write(&self, req: &Request) -> Result<(), String> {
        let mut g = self.proc_.lock().await;
        let p = g.as_mut().ok_or("Speech engine isn't running")?;
        let mut line = serde_json::to_string(req).map_err(|e| e.to_string())?;
        line.push('\n');
        p.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("speech engine: {e}"))?;
        p.stdin.flush().await.map_err(|e| format!("speech engine: {e}"))
    }

    /// Kills the process (e.g. after a timeout). It is restarted on next use.
    pub async fn kill(&self) {
        let mut g = self.proc_.lock().await;
        if let Some(mut p) = g.take() {
            let _ = p.child.start_kill();
        }
        self.status.lock().running = false;
        self.status.lock().model_path = None;
        for (_, tx) in self.pending.lock().drain() {
            let _ = tx.send(Err("Speech engine reset".into()));
        }
    }

    pub async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::Relaxed);
        let _ = self.write(&Request::Shutdown).await;
        self.kill().await;
    }

    async fn ensure_running(self: &Arc<Self>) -> Result<(), String> {
        if self.proc_.lock().await.is_none() {
            self.spawn().await?;
        }
        Ok(())
    }

    /// Loads a model and keeps it loaded. Falls back to the CPU build if
    /// the GPU build fails to load.
    pub async fn load(self: &Arc<Self>, model_path: &str) -> Result<bool, String> {
        *self.want_model.lock() = Some(model_path.to_string());
        if self.is_loaded(model_path) {
            return Ok(self.status.lock().gpu);
        }
        let result = self.load_once(model_path, !self.cpu_only.load(Ordering::Relaxed)).await;
        match result {
            Ok(gpu) => Ok(gpu),
            Err(e) if !self.cpu_only.load(Ordering::Relaxed) => {
                log::warn!("GPU speech engine failed ({e}); switching to CPU");
                self.cpu_only.store(true, Ordering::Relaxed);
                self.kill().await;
                self.load_once(model_path, false).await
            }
            Err(e) => Err(e),
        }
    }

    async fn load_once(self: &Arc<Self>, model_path: &str, use_gpu: bool) -> Result<bool, String> {
        self.ensure_running().await?;
        let (tx, rx) = oneshot::channel();
        *self.load_waiter.lock() = Some(tx);
        self.write(&Request::Load {
            model_path: model_path.into(),
            use_gpu,
        })
        .await?;
        let timeout = Duration::from_millis(wirl_core::pipeline::MODEL_LOAD_TIMEOUT_MS);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok((gpu, ms)))) => {
                log::info!("model loaded in {ms} ms (gpu: {gpu})");
                self.status.lock().last_error = None;
                Ok(gpu)
            }
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(_)) => Err("Speech engine stopped while loading".into()),
            Err(_) => {
                self.kill().await;
                Err("Loading the speech model timed out".into())
            }
        }
    }

    /// Transcribes one chunk. On timeout the engine is reset.
    pub async fn transcribe(
        self: &Arc<Self>,
        samples: &[f32],
        language: &str,
        prompt: &str,
        audio_ctx: i32,
        timeout: Duration,
    ) -> Result<(String, u64), String> {
        let want = self.want_model.lock().clone();
        if let Some(m) = want {
            if !self.is_loaded(&m) {
                self.load(&m).await?;
            }
        } else {
            return Err("No speech model selected".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(id, tx);
        let req = Request::Transcribe {
            id,
            pcm16_b64: encode_pcm(samples),
            language: language.into(),
            initial_prompt: prompt.into(),
            audio_ctx,
        };
        let start = Instant::now();
        if let Err(e) = self.write(&req).await {
            self.pending.lock().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("Speech engine stopped".into()),
            Err(_) => {
                self.pending.lock().remove(&id);
                log::error!(
                    "transcription timed out after {} ms; resetting engine",
                    start.elapsed().as_millis()
                );
                self.kill().await;
                // Reload in the background so the next dictation is ready.
                tokio::spawn(self.clone().reload_later(Duration::ZERO));
                Err("Transcription timed out".into())
            }
        }
    }
}

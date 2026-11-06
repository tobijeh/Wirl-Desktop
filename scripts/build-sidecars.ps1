# Builds both Whisper sidecars for Windows x64 and puts them where Tauri
# bundles them (src-tauri/binaries/<name>-<target-triple>.exe).
#
#   wirl-whisper      Vulkan GPU build (whisper.cpp uses the CPU if no GPU)
#   wirl-whisper-cpu  CPU-only build, used if the Vulkan build can't start
#
# Both are Release builds with AVX2 and no "native" CPU tuning, so they run
# on any x64 machine with AVX2, not just the build machine.
#
# Needs: Rust (MSVC), CMake, LLVM/clang (bindgen), and for Vulkan the
# Vulkan SDK with VULKAN_SDK set.
param([switch]$CpuOnly)
$ErrorActionPreference = "Stop"

$env:GGML_NATIVE = "OFF"
$env:GGML_AVX = "ON"
$env:GGML_AVX2 = "ON"
$env:GGML_FMA = "ON"
$env:GGML_F16C = "ON"
$env:GGML_AVX512 = "OFF"

$triple = "x86_64-pc-windows-msvc"
$out = Join-Path $PSScriptRoot "..\src-tauri\binaries"
New-Item -ItemType Directory -Force -Path $out | Out-Null

Write-Host "Building CPU sidecar..."
cargo build --release -p wirl-whisper --target $triple --target-dir target/sidecar-cpu
if ($LASTEXITCODE -ne 0) { throw "CPU sidecar build failed" }
Copy-Item "target/sidecar-cpu/$triple/release/wirl-whisper.exe" (Join-Path $out "wirl-whisper-cpu-$triple.exe") -Force

if ($CpuOnly) {
  Copy-Item "target/sidecar-cpu/$triple/release/wirl-whisper.exe" (Join-Path $out "wirl-whisper-$triple.exe") -Force
  Write-Host "CPU-only: used the CPU build for both sidecars."
  exit 0
}

Write-Host "Building Vulkan sidecar..."
cargo build --release -p wirl-whisper --features vulkan --target $triple --target-dir target/sidecar-vulkan
if ($LASTEXITCODE -ne 0) { throw "Vulkan sidecar build failed" }
Copy-Item "target/sidecar-vulkan/$triple/release/wirl-whisper.exe" (Join-Path $out "wirl-whisper-$triple.exe") -Force
Write-Host "Sidecars ready in src-tauri/binaries"

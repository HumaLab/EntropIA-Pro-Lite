//! Which GPU runs the two local vision engines (GLM-OCR for OCRH, Qwen3-VL
//! for OCR correction) and which llama.cpp build drives it.
//!
//! - NVIDIA: CUDA, as before (Windows downloads it, the Linux `.deb` bundles
//!   it). No VRAM minimum, unchanged behavior.
//! - AMD Radeon on a chip llama.cpp's HIP build was compiled for: HIP
//!   (ROCm). On Linux only when the system has the ROCm runtime, which the
//!   release build links but does not ship.
//! - Any other AMD or Intel GPU with dedicated memory: Vulkan.
//! - Apple Silicon: Metal, on unified memory.
//!
//! Non-NVIDIA GPUs are offered only with enough memory for the engine:
//! [`GLM_OCR_MIN_VRAM`] and [`QWEN_MIN_VRAM`]. Integrated GPUs report next to
//! no dedicated memory, so they never qualify.

use std::sync::OnceLock;

const MIB: u64 = 1024 * 1024;

/// GLM-OCR needs ~3 GB (0.9B Q8 + projector + 4 slots of 4k). "4 GB" cards
/// report a little less than 4 GiB (an RX 560 gives 4095 MiB), so the floor
/// sits at 3.5 GiB.
pub(crate) const GLM_OCR_MIN_VRAM: u64 = 3584 * MIB;
/// Qwen3-VL 4B Q4_K_M with one 16k slot holds ~5 GB: a nominal 6 GB card.
pub(crate) const QWEN_MIN_VRAM: u64 = 5632 * MIB;

pub(crate) const VENDOR_AMD: u32 = 0x1002;
pub(crate) const VENDOR_INTEL: u32 = 0x8086;

/// The llama.cpp build a machine runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Cuda,
    Hip,
    Vulkan,
    Metal,
}

impl Backend {
    /// Name stored in the managed install's marker file.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Backend::Cuda => "cuda",
            Backend::Hip => "hip",
            Backend::Vulkan => "vulkan",
            Backend::Metal => "metal",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Backend> {
        match value.trim() {
            "cuda" => Some(Backend::Cuda),
            "hip" => Some(Backend::Hip),
            "vulkan" => Some(Backend::Vulkan),
            "metal" => Some(Backend::Metal),
            _ => None,
        }
    }

    /// Device-id prefix `llama-server --list-devices` prints for this build.
    pub(crate) fn device_prefix(self) -> &'static str {
        match self {
            Backend::Cuda => "CUDA",
            Backend::Hip => "ROCm",
            Backend::Vulkan => "Vulkan",
            Backend::Metal => "MTL",
        }
    }
}

/// One local vision engine, for its memory floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Engine {
    GlmOcr,
    /// PaddleOCR-VL's GGUF build on llama-server: ~2.8 GB (BF16 model and
    /// projector, 4 slots of 10k), the same "4 GB" card as GLM-OCR.
    PaddleVl,
    Qwen,
}

impl Engine {
    pub(crate) fn min_vram(self) -> u64 {
        match self {
            Engine::GlmOcr | Engine::PaddleVl => GLM_OCR_MIN_VRAM,
            Engine::Qwen => QWEN_MIN_VRAM,
        }
    }

    /// The floor as the nominal card size users read on the box.
    pub(crate) fn min_vram_gb(self) -> u32 {
        match self {
            Engine::GlmOcr | Engine::PaddleVl => 4,
            Engine::Qwen => 6,
        }
    }
}

/// A display adapter as the OS reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Adapter {
    pub vendor_id: u32,
    pub device_id: u32,
    /// Dedicated video memory in bytes (0 when unknown).
    pub dedicated_vram: u64,
    pub name: String,
}

/// The GPU the local engines would run on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Gpu {
    pub backend: Backend,
    /// Usable memory in bytes; `None` for NVIDIA, which has no floor.
    pub vram: Option<u64>,
    /// Adapter name, to pick the same device among llama.cpp's.
    pub name: Option<String>,
}

impl Gpu {
    pub(crate) fn fits(&self, engine: Engine) -> bool {
        self.vram.is_none_or(|vram| vram >= engine.min_vram())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    Windows,
    Linux,
    MacArm,
    Other,
}

impl Platform {
    pub(crate) fn current() -> Platform {
        if cfg!(all(windows, target_arch = "x86_64")) {
            Platform::Windows
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Platform::Linux
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Platform::MacArm
        } else {
            Platform::Other
        }
    }
}

/// What the machine offers, gathered once.
#[derive(Clone, Debug, Default)]
pub(crate) struct Hardware {
    pub has_nvidia: bool,
    pub adapters: Vec<Adapter>,
    /// Linux: the ROCm runtime (`libamdhip64`) is installed.
    pub rocm_runtime: bool,
    /// macOS: physical memory in bytes.
    pub unified_memory: Option<u64>,
}

/// The GPU to use, or `None` when the local engines have none to run on.
pub(crate) fn choose(platform: Platform, hardware: &Hardware) -> Option<Gpu> {
    if platform == Platform::MacArm {
        let memory = hardware.unified_memory?;
        return (memory >= GLM_OCR_MIN_VRAM).then_some(Gpu {
            backend: Backend::Metal,
            vram: Some(memory),
            name: None,
        });
    }
    if hardware.has_nvidia {
        return Some(Gpu {
            backend: Backend::Cuda,
            vram: None,
            name: None,
        });
    }
    if !matches!(platform, Platform::Windows | Platform::Linux) {
        return None;
    }
    let adapter = hardware
        .adapters
        .iter()
        .filter(|adapter| matches!(adapter.vendor_id, VENDOR_AMD | VENDOR_INTEL))
        .filter(|adapter| adapter.dedicated_vram >= GLM_OCR_MIN_VRAM)
        .max_by_key(|adapter| adapter.dedicated_vram)?;
    let hip = adapter.vendor_id == VENDOR_AMD
        && hip_target(platform, adapter.device_id).is_some()
        && (platform == Platform::Windows || hardware.rocm_runtime);
    Some(Gpu {
        backend: if hip { Backend::Hip } else { Backend::Vulkan },
        vram: Some(adapter.dedicated_vram),
        name: Some(adapter.name.clone()),
    })
}

/// The `gfx` target of an AMD PCI device id, when llama.cpp's HIP build for
/// `platform` carries kernels for it (`gpu_targets` of the `windows-hip` and
/// `ubuntu-22-rocm` jobs in llama.cpp's release.yml at tag b8783). Ids come
/// from the amdgpu driver's tables. A card missing here gets Vulkan; one
/// wrongly listed is caught after install, when the HIP build lists no
/// device and the install falls back to Vulkan.
pub(crate) fn hip_target(platform: Platform, device_id: u32) -> Option<&'static str> {
    let target = match device_id {
        0x73A0..=0x73AF | 0x73BF => "gfx1030", // Navi 21: RX 6800/6900
        0x73C0..=0x73DF => "gfx1031",          // Navi 22: RX 6700
        0x73E0..=0x73FF => "gfx1032",          // Navi 23: RX 6600
        0x7440..=0x744F => "gfx1100",          // Navi 31: RX 7900
        0x7470..=0x747F => "gfx1101",          // Navi 32: RX 7700/7800
        0x7480..=0x748F => "gfx1102",          // Navi 33: RX 7600
        0x150E => "gfx1150",                   // Strix Point
        0x1586 => "gfx1151",                   // Strix Halo
        0x7590..=0x759F => "gfx1200",          // Navi 44: RX 9060
        0x7550..=0x755F => "gfx1201",          // Navi 48: RX 9070
        _ => return None,
    };
    // The Linux ROCm build skips the RDNA2 cards below Navi 21.
    let linux_missing = matches!(target, "gfx1031" | "gfx1032");
    match platform {
        Platform::Windows => Some(target),
        Platform::Linux if !linux_missing => Some(target),
        _ => None,
    }
}

/// The device id to pass `--device` from `llama-server --list-devices`
/// output: the `backend`'s device named like `preferred`, else the one with
/// the most free memory. Without it llama.cpp splits a model across every
/// device it sees, integrated GPUs included (an Intel UHD lists its shared
/// memory as 16 GB with 0 free).
pub(crate) fn pick_device(
    list_output: &str,
    backend: Backend,
    preferred: Option<&str>,
) -> Option<String> {
    let devices: Vec<(String, String, u64)> = list_output
        .lines()
        .filter_map(|line| parse_device_line(line.trim()))
        .filter(|(id, _, _)| id.starts_with(backend.device_prefix()))
        .collect();
    if let Some(preferred) = preferred.map(normalize_name) {
        if let Some((id, _, _)) = devices
            .iter()
            .find(|(_, name, _)| normalize_name(name) == preferred)
        {
            return Some(id.clone());
        }
    }
    devices
        .into_iter()
        .max_by_key(|(_, _, free)| *free)
        .map(|(id, _, _)| id)
}

/// `Vulkan0: AMD Radeon(TM) RX 560 Series (4096 MiB, 3463 MiB free)`.
fn parse_device_line(line: &str) -> Option<(String, String, u64)> {
    let (id, rest) = line.split_once(": ")?;
    if id.is_empty() || id.contains(' ') {
        return None;
    }
    let (name, memory) = rest.rsplit_once(" (")?;
    let free = memory
        .split(", ")
        .nth(1)?
        .strip_suffix(" MiB free)")?
        .parse::<u64>()
        .ok()?;
    Some((id.to_string(), name.to_string(), free))
}

fn normalize_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// Whether `output` lists at least one `backend` device.
pub(crate) fn lists_device(output: &str, backend: Backend) -> bool {
    pick_device(output, backend, None).is_some()
}

/// This machine's GPU, probed once per session: the hardware does not
/// change while the app runs.
pub(crate) fn detected() -> Option<Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    GPU.get_or_init(|| {
        let platform = Platform::current();
        let hardware = Hardware {
            has_nvidia: platform != Platform::MacArm && crate::llm::has_nvidia_gpu(),
            adapters: platform::adapters(),
            rocm_runtime: platform::rocm_runtime(),
            unified_memory: platform::unified_memory(),
        };
        let gpu = choose(platform, &hardware);
        log::info!(
            target: "llama_gpu",
            "adapters={:?} rocm={} gpu={gpu:?}",
            hardware.adapters,
            hardware.rocm_runtime
        );
        gpu
    })
    .clone()
}

/// Whether `engine` has a GPU with enough memory on this machine.
pub(crate) fn engine_runs(engine: Engine) -> bool {
    detected().is_some_and(|gpu| gpu.fits(engine))
}

#[cfg(windows)]
mod platform {
    use super::Adapter;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
    };

    /// Hardware adapters from DXGI, the software rasterizer left out.
    pub(super) fn adapters() -> Vec<Adapter> {
        let mut found = Vec::new();
        // SAFETY: plain DXGI calls; every interface is released on drop.
        let factory: IDXGIFactory1 = match unsafe { CreateDXGIFactory1() } {
            Ok(factory) => factory,
            Err(error) => {
                log::warn!(target: "llama_gpu", "CreateDXGIFactory1 failed: {error}");
                return found;
            }
        };
        let mut index = 0;
        while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
            index += 1;
            let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
                continue;
            };
            if desc.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }
            let len = desc
                .Description
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(desc.Description.len());
            found.push(Adapter {
                vendor_id: desc.VendorId,
                device_id: desc.DeviceId,
                dedicated_vram: desc.DedicatedVideoMemory as u64,
                name: String::from_utf16_lossy(&desc.Description[..len]),
            });
        }
        found
    }

    pub(super) fn rocm_runtime() -> bool {
        false
    }

    pub(super) fn unified_memory() -> Option<u64> {
        None
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::Adapter;
    use std::path::Path;

    /// DRM cards from sysfs. Only amdgpu reports VRAM there
    /// (`mem_info_vram_total`), so other vendors show 0 and do not qualify.
    pub(super) fn adapters() -> Vec<Adapter> {
        let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with("card") && !name.contains('-')
            })
            .filter_map(|entry| {
                let device = entry.path().join("device");
                let hex = |file: &str| {
                    let text = std::fs::read_to_string(device.join(file)).ok()?;
                    u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok()
                };
                let vram = std::fs::read_to_string(device.join("mem_info_vram_total"))
                    .ok()
                    .and_then(|text| text.trim().parse::<u64>().ok())
                    .unwrap_or(0);
                Some(Adapter {
                    vendor_id: hex("vendor")?,
                    device_id: hex("device")?,
                    dedicated_vram: vram,
                    name: String::new(),
                })
            })
            .collect()
    }

    /// The ROCm runtime the release's `libggml-hip.so` links against.
    pub(super) fn rocm_runtime() -> bool {
        [
            "/opt/rocm/lib",
            "/usr/lib/x86_64-linux-gnu",
            "/usr/lib64",
            "/usr/lib",
        ]
        .iter()
        .any(|dir| {
            std::fs::read_dir(Path::new(dir)).is_ok_and(|entries| {
                entries.flatten().any(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("libamdhip64.so")
                })
            })
        })
    }

    pub(super) fn unified_memory() -> Option<u64> {
        None
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Adapter;

    pub(super) fn adapters() -> Vec<Adapter> {
        Vec::new()
    }

    pub(super) fn rocm_runtime() -> bool {
        false
    }

    /// `hw.memsize`: Apple Silicon GPUs work on the system's memory.
    pub(super) fn unified_memory() -> Option<u64> {
        let output = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout).trim().parse().ok()
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod platform {
    use super::Adapter;

    pub(super) fn adapters() -> Vec<Adapter> {
        Vec::new()
    }

    pub(super) fn rocm_runtime() -> bool {
        false
    }

    pub(super) fn unified_memory() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * MIB;

    fn adapter(vendor_id: u32, device_id: u32, dedicated_vram: u64, name: &str) -> Adapter {
        Adapter {
            vendor_id,
            device_id,
            dedicated_vram,
            name: name.to_string(),
        }
    }

    fn hardware(adapters: Vec<Adapter>) -> Hardware {
        Hardware {
            adapters,
            ..Hardware::default()
        }
    }

    /// The owner's test machine: an RX 560 (Polaris, no HIP kernels) next to
    /// an Intel UHD 630, as DXGI reports them.
    fn rx560_and_uhd() -> Hardware {
        hardware(vec![
            adapter(VENDOR_INTEL, 0x3E92, 128 * MIB, "Intel(R) UHD Graphics 630"),
            adapter(
                VENDOR_AMD,
                0x67FF,
                4095 * MIB,
                "AMD Radeon(TM) RX 560 Series",
            ),
        ])
    }

    #[test]
    fn nvidia_keeps_cuda_with_no_memory_floor() {
        let mut machine = rx560_and_uhd();
        machine.has_nvidia = true;
        let gpu = choose(Platform::Windows, &machine).unwrap();
        assert_eq!(gpu.backend, Backend::Cuda);
        assert!(gpu.fits(Engine::GlmOcr) && gpu.fits(Engine::Qwen));
        assert_eq!(
            choose(Platform::Linux, &machine).unwrap().backend,
            Backend::Cuda
        );
    }

    #[test]
    fn a_4_gb_radeon_without_hip_kernels_gets_vulkan_for_glm_only() {
        let gpu = choose(Platform::Windows, &rx560_and_uhd()).unwrap();
        assert_eq!(gpu.backend, Backend::Vulkan);
        assert_eq!(gpu.name.as_deref(), Some("AMD Radeon(TM) RX 560 Series"));
        assert!(gpu.fits(Engine::GlmOcr));
        assert!(!gpu.fits(Engine::Qwen));
    }

    #[test]
    fn a_supported_radeon_gets_hip_on_windows_and_on_linux_with_rocm() {
        let rx7900 = hardware(vec![adapter(VENDOR_AMD, 0x744C, 24 * GIB, "RX 7900 XTX")]);
        let gpu = choose(Platform::Windows, &rx7900).unwrap();
        assert_eq!(gpu.backend, Backend::Hip);
        assert!(gpu.fits(Engine::Qwen));

        assert_eq!(
            choose(Platform::Linux, &rx7900).unwrap().backend,
            Backend::Vulkan,
            "no ROCm runtime installed"
        );
        let mut with_rocm = rx7900.clone();
        with_rocm.rocm_runtime = true;
        assert_eq!(
            choose(Platform::Linux, &with_rocm).unwrap().backend,
            Backend::Hip
        );
    }

    #[test]
    fn rx_6600_has_hip_kernels_on_windows_only() {
        assert_eq!(hip_target(Platform::Windows, 0x73FF), Some("gfx1032"));
        assert_eq!(hip_target(Platform::Linux, 0x73FF), None);
        assert_eq!(hip_target(Platform::Linux, 0x73BF), Some("gfx1030"));
        assert_eq!(hip_target(Platform::Windows, 0x67FF), None);
    }

    #[test]
    fn an_intel_arc_gets_vulkan() {
        let arc = hardware(vec![adapter(
            VENDOR_INTEL,
            0xE20B,
            12 * GIB,
            "Intel(R) Arc(TM) B580",
        )]);
        let gpu = choose(Platform::Windows, &arc).unwrap();
        assert_eq!(gpu.backend, Backend::Vulkan);
        assert!(gpu.fits(Engine::Qwen));
    }

    #[test]
    fn small_or_integrated_gpus_are_not_offered() {
        assert_eq!(
            choose(
                Platform::Windows,
                &hardware(vec![adapter(VENDOR_AMD, 0x699F, 2 * GIB, "RX 550")])
            ),
            None
        );
        assert_eq!(
            choose(
                Platform::Windows,
                &hardware(vec![adapter(VENDOR_INTEL, 0x3E92, 128 * MIB, "UHD 630")])
            ),
            None
        );
        assert_eq!(choose(Platform::Windows, &Hardware::default()), None);
    }

    #[test]
    fn apple_silicon_runs_metal_on_unified_memory() {
        let mac = |gb: u64| Hardware {
            unified_memory: Some(gb * GIB),
            ..Hardware::default()
        };
        let gpu = choose(Platform::MacArm, &mac(16)).unwrap();
        assert_eq!(gpu.backend, Backend::Metal);
        assert!(gpu.fits(Engine::Qwen));
        assert_eq!(choose(Platform::MacArm, &Hardware::default()), None);
    }

    #[test]
    fn other_platforms_only_run_nvidia() {
        let mut machine = rx560_and_uhd();
        assert_eq!(choose(Platform::Other, &machine), None);
        machine.has_nvidia = true;
        assert_eq!(
            choose(Platform::Other, &machine).unwrap().backend,
            Backend::Cuda
        );
    }

    /// `llama-server --list-devices` of the Vulkan build on the RX 560 + UHD
    /// 630 machine.
    const RX560_LIST: &str = "load_backend: loaded Vulkan backend from C:\\x\\ggml-vulkan.dll\n\
Available devices:\n  Vulkan0: AMD Radeon(TM) RX 560 Series (4096 MiB, 3463 MiB free)\n  \
Vulkan1: Intel(R) UHD Graphics 630 (16447 MiB, 0 MiB free)\n";

    #[test]
    fn the_device_named_like_the_adapter_is_picked() {
        assert_eq!(
            pick_device(
                RX560_LIST,
                Backend::Vulkan,
                Some("AMD Radeon(TM) RX 560 Series")
            ),
            Some("Vulkan0".to_string())
        );
    }

    #[test]
    fn without_a_name_the_device_with_most_free_memory_wins() {
        assert_eq!(
            pick_device(RX560_LIST, Backend::Vulkan, None),
            Some("Vulkan0".to_string())
        );
        assert_eq!(
            pick_device(RX560_LIST, Backend::Vulkan, Some("Some other card")),
            Some("Vulkan0".to_string())
        );
    }

    #[test]
    fn a_hip_build_with_no_rocm_device_lists_none() {
        // The HIP build on the RX 560: its backend does not load.
        let hip_on_polaris =
            "load_backend: loaded CPU backend from C:\\x\\ggml-cpu.dll\nAvailable devices:\n";
        assert!(!lists_device(hip_on_polaris, Backend::Hip));
        assert!(!lists_device(RX560_LIST, Backend::Hip));
        assert!(lists_device(
            "Available devices:\n  ROCm0: AMD Radeon RX 7900 XTX (24560 MiB, 24000 MiB free)\n",
            Backend::Hip
        ));
    }

    #[test]
    fn the_marker_round_trips() {
        for backend in [Backend::Cuda, Backend::Hip, Backend::Vulkan, Backend::Metal] {
            assert_eq!(Backend::parse(backend.as_str()), Some(backend));
        }
        assert_eq!(Backend::parse("rocm"), None);
    }
}

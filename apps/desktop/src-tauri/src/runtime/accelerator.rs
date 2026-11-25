//! Which runtime pack this machine should download: the CPU pack every
//! machine can run, or the CUDA pack for an NVIDIA GPU with a recent enough
//! driver.
//!
//! The CUDA pack is published under its own bootstrap platform
//! (`{platform}-cuda`, see `manifest::cuda_release_platform`), so app builds
//! that predate it never select it. Inside the downloaded pack the runtime
//! manifest keeps the plain OS platform: the runtime runs on that OS, and the
//! discovery, reuse and sweep rules that compare platforms keep working.

use std::process::Command;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeAccelerator {
    Cpu,
    Cuda,
}

/// Oldest NVIDIA driver (major version) that runs the CUDA pack on Linux. The
/// pack ships PaddlePaddle built for CUDA 12.9, which needs driver 575.51.03
/// or newer there; an older driver would download several GB only to fall
/// back to the CPU at run time, so it gets the CPU pack instead.
pub const MIN_CUDA_DRIVER_MAJOR: u32 = 575;

/// The same floor on Windows, where CUDA 12.9 needs driver 576.02 or newer.
pub const MIN_CUDA_DRIVER_MAJOR_WINDOWS: u32 = 576;

/// The driver floor for one OS, named as `std::env::consts::OS` names it.
fn min_cuda_driver_major(os: &str) -> u32 {
    if os == "windows" {
        MIN_CUDA_DRIVER_MAJOR_WINDOWS
    } else {
        MIN_CUDA_DRIVER_MAJOR
    }
}

/// Forces the choice for testing and support: `cpu` or `cuda`. Anything else
/// (or unset) leaves the decision to the GPU probe.
pub const ACCELERATOR_OVERRIDE_ENV: &str = "ENTROPIA_RUNTIME_ACCELERATOR";

/// The accelerator for one `nvidia-smi --query-gpu=driver_version` output on
/// this OS: CUDA when the first GPU's driver is new enough, CPU otherwise (no
/// NVIDIA GPU, no driver, or output that does not parse).
pub fn accelerator_for_driver_output(output: Option<&str>) -> RuntimeAccelerator {
    accelerator_for_driver_output_on(std::env::consts::OS, output)
}

/// `accelerator_for_driver_output` for a named OS.
fn accelerator_for_driver_output_on(os: &str, output: Option<&str>) -> RuntimeAccelerator {
    let Some(first_line) = output.and_then(|text| text.lines().find(|l| !l.trim().is_empty()))
    else {
        return RuntimeAccelerator::Cpu;
    };
    let major = first_line
        .trim()
        .split('.')
        .next()
        .and_then(|part| part.trim().parse::<u32>().ok());
    match major {
        Some(major) if major >= min_cuda_driver_major(os) => RuntimeAccelerator::Cuda,
        _ => RuntimeAccelerator::Cpu,
    }
}

/// The override, when `ENTROPIA_RUNTIME_ACCELERATOR` names one.
pub fn accelerator_override(value: Option<&str>) -> Option<RuntimeAccelerator> {
    match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("cpu") => Some(RuntimeAccelerator::Cpu),
        Some("cuda") => Some(RuntimeAccelerator::Cuda),
        _ => None,
    }
}

fn nvidia_driver_output() -> Option<String> {
    let mut command = Command::new("nvidia-smi");
    command.args(["--query-gpu=driver_version", "--format=csv,noheader"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// This machine's accelerator, probed once per process: the runtime status
/// is inspected often and `nvidia-smi` takes a noticeable moment.
pub fn detect_runtime_accelerator() -> RuntimeAccelerator {
    static DETECTED: OnceLock<RuntimeAccelerator> = OnceLock::new();
    *DETECTED.get_or_init(|| {
        if let Some(forced) =
            accelerator_override(std::env::var(ACCELERATOR_OVERRIDE_ENV).ok().as_deref())
        {
            log::info!(target: "runtime", "runtime accelerator forced to {forced:?}");
            return forced;
        }
        let output = nvidia_driver_output();
        let accelerator = accelerator_for_driver_output(output.as_deref());
        log::info!(
            target: "runtime",
            "runtime accelerator: {accelerator:?} (nvidia driver: {})",
            output.as_deref().map(str::trim).unwrap_or("none")
        );
        accelerator
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recent_nvidia_driver_selects_cuda() {
        assert_eq!(
            accelerator_for_driver_output(Some("580.173.02\n")),
            RuntimeAccelerator::Cuda
        );
        assert_eq!(
            accelerator_for_driver_output_on("linux", Some("575.51.03")),
            RuntimeAccelerator::Cuda
        );
    }

    #[test]
    fn an_old_driver_keeps_the_cpu_pack() {
        assert_eq!(
            accelerator_for_driver_output(Some("560.35.03\n")),
            RuntimeAccelerator::Cpu
        );
    }

    #[test]
    fn no_nvidia_gpu_or_unreadable_output_keeps_the_cpu_pack() {
        assert_eq!(accelerator_for_driver_output(None), RuntimeAccelerator::Cpu);
        assert_eq!(
            accelerator_for_driver_output(Some("")),
            RuntimeAccelerator::Cpu
        );
        assert_eq!(
            accelerator_for_driver_output(Some("NVIDIA-SMI has failed")),
            RuntimeAccelerator::Cpu
        );
    }

    #[test]
    fn windows_needs_the_windows_driver_that_ships_cuda_12_9() {
        // CUDA 12.9 needs driver 575.51.03 on Linux but 576.02 on Windows.
        assert_eq!(
            accelerator_for_driver_output_on("windows", Some("576.02")),
            RuntimeAccelerator::Cuda
        );
        assert_eq!(
            accelerator_for_driver_output_on("windows", Some("575.64")),
            RuntimeAccelerator::Cpu
        );
        // A Kepler card stuck on the last driver it gets keeps the CPU pack.
        assert_eq!(
            accelerator_for_driver_output_on("windows", Some("475.14")),
            RuntimeAccelerator::Cpu
        );
        assert_eq!(
            accelerator_for_driver_output_on("linux", Some("575.51.03")),
            RuntimeAccelerator::Cuda
        );
    }

    #[test]
    fn several_gpus_are_judged_by_the_first_one() {
        assert_eq!(
            accelerator_for_driver_output(Some("\n580.173.02\n580.173.02\n")),
            RuntimeAccelerator::Cuda
        );
    }

    #[test]
    fn the_override_accepts_cpu_and_cuda_only() {
        assert_eq!(
            accelerator_override(Some("cpu")),
            Some(RuntimeAccelerator::Cpu)
        );
        assert_eq!(
            accelerator_override(Some(" CUDA ")),
            Some(RuntimeAccelerator::Cuda)
        );
        assert_eq!(accelerator_override(Some("rocm")), None);
        assert_eq!(accelerator_override(None), None);
    }
}

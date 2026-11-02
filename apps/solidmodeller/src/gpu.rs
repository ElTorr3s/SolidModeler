//! Picks the graphics adapter. Vulkan is preferred: it is what Omarchy ships
//! drivers for and what the viewport is tested on most. OpenGL still works
//! (the viewport falls back to shader sRGB encoding there), and a software
//! renderer is the last resort. `WGPU_BACKEND` still limits the choice.

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup};
use std::sync::Arc;

/// eframe's wgpu settings with our adapter choice.
pub fn configure(mut config: WgpuConfiguration) -> WgpuConfiguration {
    if let WgpuSetup::CreateNew(setup) = &mut config.wgpu_setup {
        setup.native_adapter_selector = Some(Arc::new(|adapters, surface| {
            let usable = adapters
                .iter()
                .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)));
            usable
                .max_by_key(|a| {
                    let info = a.get_info();
                    rank(info.backend, info.device_type)
                })
                .cloned()
                .ok_or_else(|| no_adapter(adapters))
        }));
    }
    config
}

/// Higher is better: real GPUs before software renderers, then Vulkan
/// before other backends, then discrete before integrated.
fn rank(backend: wgpu::Backend, device_type: wgpu::DeviceType) -> (bool, bool, u8) {
    use wgpu::DeviceType::*;
    let kind = match device_type {
        DiscreteGpu => 4,
        IntegratedGpu => 3,
        VirtualGpu => 2,
        Cpu => 1,
        Other => 0,
    };
    (kind >= 2, backend == wgpu::Backend::Vulkan, kind)
}

fn no_adapter(adapters: &[wgpu::Adapter]) -> String {
    let mut msg = String::from("no graphics adapter can draw to this window.");
    if adapters.is_empty() {
        msg += " Install a Vulkan driver (vulkan-radeon, vulkan-intel or nvidia-utils) and check `vulkaninfo --summary`.";
    } else {
        msg += " Found:";
        for a in adapters {
            let info = a.get_info();
            msg += &format!(" {} ({:?});", info.name, info.backend);
        }
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_hardware_then_vulkan_then_discrete() {
        use wgpu::{Backend::*, DeviceType::*};
        let order = [
            (Vulkan, DiscreteGpu),
            (Vulkan, IntegratedGpu),
            (Gl, DiscreteGpu),
            (Gl, IntegratedGpu),
            (Vulkan, Cpu),
            (Gl, Cpu),
            (Gl, Other),
        ];
        for pair in order.windows(2) {
            let ((better, better_type), (worse, worse_type)) = (pair[0], pair[1]);
            assert!(
                rank(better, better_type) > rank(worse, worse_type),
                "{pair:?}"
            );
        }
    }

    #[test]
    fn selector_is_installed() {
        let config = configure(WgpuConfiguration::default());
        let WgpuSetup::CreateNew(setup) = config.wgpu_setup else {
            panic!("expected a new wgpu setup");
        };
        let select = setup.native_adapter_selector.expect("selector");
        let err = select(&[], None).unwrap_err();
        assert!(err.contains("vulkan"), "{err}");
    }
}
